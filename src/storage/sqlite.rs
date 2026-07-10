#![allow(clippy::too_many_arguments)]
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::models::{
    IndexedFile, ReferenceWithPath, Symbol, SymbolReference, SymbolWithPath,
};

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;


/// Sanitize a query string for FTS5 MATCH to prevent query syntax errors.
/// FTS5 has special characters: " * - ( ) : AND OR NOT NEAR
/// This function escapes double quotes and wraps the query in quotes for phrase matching.
fn sanitize_fts5_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Escape double quotes by doubling them
    let escaped = trimmed.replace('"', "\"\"");

    // Wrap in double quotes for exact phrase matching
    // This disables special operators and treats the input as a literal phrase
    format!("\"{}\"", escaped)
}

/// Query performance metrics
#[derive(Debug, Clone, Default)]
pub struct QueryMetrics {
    pub total_queries: Arc<AtomicU64>,
    pub slow_queries: Arc<AtomicU64>,
    pub total_query_time_ms: Arc<AtomicU64>,
}

impl QueryMetrics {
    pub fn new() -> Self {
        Self {
            total_queries: Arc::new(AtomicU64::new(0)),
            slow_queries: Arc::new(AtomicU64::new(0)),
            total_query_time_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record_query(&self, duration_ms: u64) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
        self.total_query_time_ms
            .fetch_add(duration_ms, Ordering::Relaxed);

        // Queries over 100ms are considered slow
        if duration_ms > 100 {
            self.slow_queries.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("Slow query detected: {}ms", duration_ms);
        }
    }

}

/// SQLite storage for file tracking and symbol graph
#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    metrics: QueryMetrics,
}

// Многие методы SqliteStorage являются частью публичного API, который
// используется языковыми сервисами и MCP-инструментами через daemon/tools.rs.
// Некоторые из них пока не вызываются напрямую, но составляют контракт API.
#[allow(dead_code)]
impl SqliteStorage {
    /// Create a new SQLite storage instance
    pub async fn new(db_path: &str) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(db_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let connection_string = format!("sqlite:{}?mode=rwc", db_path);

        // PRAGMAs that are *per-connection* must be set via SqliteConnectOptions, not
        // executed once on the pool. Previously busy_timeout/synchronous/cache_size were
        // set with `sqlx::query(...).execute(&pool)`, which only configured a single
        // connection — every other connection in the pool defaulted to busy_timeout=0
        // and returned SQLITE_BUSY immediately on any write contention.
        let opts = SqliteConnectOptions::from_str(&connection_string)?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(30))
            .pragma("cache_size", "-64000")
            .pragma("temp_store", "MEMORY")
            .pragma("mmap_size", "30000000000");

        // Feature 015: Enhanced connection pooling.
        // SQLite under WAL allows many concurrent readers but only one writer at a time —
        // 64 connections was overkill and amplified write contention. 16 is plenty.
        let pool = SqlitePoolOptions::new()
            .max_connections(16)
            .min_connections(2)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Some(Duration::from_secs(300)))
            .max_lifetime(Some(Duration::from_secs(1800)))
            .connect_with(opts)
            .await?;

        // page_size and auto_vacuum must be set before the first write to a fresh DB,
        // and on an existing DB they're persisted — running them as ordinary statements
        // here covers both cases without needing them on every new connection.
        sqlx::query("PRAGMA page_size = 8192")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
            .execute(&pool)
            .await?;

        Ok(Self {
            pool,
            metrics: QueryMetrics::new(),
        })
    }


    /// Get pool for transaction support
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Get query metrics
    pub fn metrics(&self) -> &QueryMetrics {
        &self.metrics
    }

    /// Helper to execute a query with metrics tracking
    async fn execute_with_metrics<'a, F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(
            &SqlitePool,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = std::result::Result<T, sqlx::Error>> + Send + 'a>,
        >,
    {
        let start = Instant::now();
        let result = f(&self.pool).await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        self.metrics.record_query(duration_ms);
        Ok(result)
    }

    /// Run migrations
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;

        tracing::info!("SQLite migrations completed");
        Ok(())
    }

    // === File Operations ===

    /// Quick connectivity check — runs SELECT 1.
    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// Check database integrity using PRAGMA integrity_check
    /// Returns Ok(()) if database is healthy, Err with details otherwise
    pub async fn check_integrity(&self) -> Result<()> {
        let result: (String,) = sqlx::query_as("PRAGMA integrity_check(1)")
            .fetch_one(&self.pool)
            .await?;

        if result.0 == "ok" {
            Ok(())
        } else {
            Err(StorageError::Database(sqlx::Error::Protocol(format!(
                "Database integrity check failed: {}",
                result.0
            ))))
        }
    }

    /// Insert or update a file record
    pub async fn upsert_file(
        &self,
        path: &str,
        last_modified: i64,
        content_hash: &str,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO files (path, last_modified, content_hash)
            VALUES (?, ?, ?)
            ON CONFLICT(path) DO UPDATE SET
                last_modified = excluded.last_modified,
                content_hash = excluded.content_hash
            RETURNING id
            "#,
        )
        .bind(path)
        .bind(last_modified)
        .bind(content_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(sqlx::Row::get(&result, "id"))
    }

    /// Get file by path
    pub async fn get_file(&self, path: &str) -> Result<Option<IndexedFile>> {
        let file = sqlx::query_as::<_, IndexedFile>(
            "SELECT id, path, last_modified, content_hash FROM files WHERE path = ?",
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(file)
    }

    /// Check if file needs reindexing
    pub async fn needs_reindex(&self, path: &str, content_hash: &str) -> Result<bool> {
        let existing = self.get_file(path).await?;

        match existing {
            Some(file) => Ok(file.content_hash != content_hash),
            None => Ok(true),
        }
    }

    /// Get all file hashes for batch comparison
    pub async fn get_all_file_hashes(
        &self,
    ) -> Result<std::collections::HashMap<String, (String, i64)>> {
        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT path, content_hash, last_modified FROM files")
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(|(p, h, m)| (p, (h, m))).collect())
    }

    /// Delete file and its symbols
    pub async fn delete_file(&self, path: &str) -> Result<()> {
        sqlx::query("DELETE FROM files WHERE path = ?")
            .bind(path)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get total count of indexed files (for health checks)
    pub async fn get_file_count(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM files")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    // === Symbol Operations ===

    /// Insert symbols for a file (deletes existing first, batched in transaction)
    pub async fn insert_symbols(&self, file_id: i64, symbols: &[Symbol]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM symbols WHERE file_id = ?")
            .bind(file_id)
            .execute(&mut *tx)
            .await?;

        if !symbols.is_empty() {
            // SQLite has a limit on bound parameters (usually 32766).
            // We bind 6 parameters per row, so ~5000 rows max.
            // Chunking by 1000 is perfectly safe.
            for chunk in symbols.chunks(1000) {
                let mut query_builder = sqlx::QueryBuilder::new(
                    "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature) ",
                );

                query_builder.push_values(chunk, |mut b, symbol| {
                    b.push_bind(file_id)
                        .push_bind(&symbol.name)
                        .push_bind(symbol.kind)
                        .push_bind(symbol.line_start)
                        .push_bind(symbol.line_end)
                        .push_bind(&symbol.signature);
                });

                let query = query_builder.build();
                query.execute(&mut *tx).await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    /// Search symbols using FTS5
    pub async fn search_symbols(&self, query: &str, limit: i32) -> Result<Vec<Symbol>> {
        let sanitized = sanitize_fts5_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let symbols = sqlx::query_as::<_, Symbol>(
            r#"
            SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature
            FROM symbols s
            WHERE s.id IN (
                SELECT rowid FROM symbols_fts WHERE symbols_fts MATCH ?
            )
            LIMIT ?
            "#,
        )
        .bind(&sanitized)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    /// Search symbols using FTS5, returning results with file paths
    pub async fn search_symbols_with_path(
        &self,
        query: &str,
        limit: i32,
    ) -> Result<Vec<SymbolWithPath>> {
        self.search_symbols_with_path_filter(query, limit, None)
            .await
    }

    pub async fn search_symbols_with_path_filter(
        &self,
        query: &str,
        limit: i32,
        path_filter: Option<&str>,
    ) -> Result<Vec<SymbolWithPath>> {
        let sanitized = sanitize_fts5_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = if let Some(_path_prefix) = path_filter {
            r#"
            SELECT s.id, s.name, s.kind, s.line_start AS line, s.line_end AS end_line,
                   s.signature, f.path AS file_path
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE s.id IN (
                SELECT rowid FROM symbols_fts WHERE symbols_fts MATCH ?
            )
            AND f.path LIKE ?
            LIMIT ?
            "#
        } else {
            r#"
            SELECT s.id, s.name, s.kind, s.line_start AS line, s.line_end AS end_line,
                   s.signature, f.path AS file_path
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE s.id IN (
                SELECT rowid FROM symbols_fts WHERE symbols_fts MATCH ?
            )
            LIMIT ?
            "#
        };

        let mut query_builder = sqlx::query_as::<_, SymbolWithPath>(sql).bind(&sanitized);

        if let Some(path_prefix) = path_filter {
            query_builder = query_builder.bind(format!("{}%", path_prefix));
        }

        let symbols = query_builder.bind(limit).fetch_all(&self.pool).await?;

        Ok(symbols)
    }

    /// Get all symbols for a file
    pub async fn get_file_symbols(&self, file_id: i64) -> Result<Vec<Symbol>> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT id, file_id, name, kind, line_start, line_end, signature FROM symbols WHERE file_id = ?"
        )
        .bind(file_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    /// Get symbol by name
    pub async fn get_symbol_by_name(&self, name: &str) -> Result<Vec<Symbol>> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT id, file_id, name, kind, line_start, line_end, signature FROM symbols WHERE name = ?"
        )
        .bind(name)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }


    /// Get file by id
    pub async fn get_file_by_id(&self, id: i64) -> Result<Option<IndexedFile>> {
        let file = sqlx::query_as::<_, IndexedFile>(
            "SELECT id, path, last_modified, content_hash FROM files WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(file)
    }

    /// Find symbol by name and file path
    pub async fn find_symbol_by_name_and_file(
        &self,
        name: &str,
        file_path: &str,
    ) -> Result<Option<Symbol>> {
        let symbol = sqlx::query_as::<_, Symbol>(
            r#"
            SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature 
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE s.name = ? AND f.path = ?
            "#,
        )
        .bind(name)
        .bind(file_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(symbol)
    }


    // === Reference Operations ===

    /// Insert references for a symbol (deletes existing first, batched in transaction)
    pub async fn insert_references(&self, symbol_id: i64, refs: &[SymbolReference]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM symbol_references WHERE source_symbol_id = ?")
            .bind(symbol_id)
            .execute(&mut *tx)
            .await?;

        if !refs.is_empty() {
            // Chunking by 1000 rows (5 parameters per row = 5000 bound parameters < 32766 limit)
            for chunk in refs.chunks(1000) {
                let mut query_builder = sqlx::QueryBuilder::new(
                    "INSERT INTO symbol_references (source_symbol_id, target_name, target_symbol_id, kind, line) "
                );

                query_builder.push_values(chunk, |mut b, r| {
                    b.push_bind(symbol_id)
                        .push_bind(&r.target_name)
                        .push_bind(r.target_symbol_id)
                        .push_bind(&r.kind)
                        .push_bind(r.line);
                });

                let query = query_builder.build();
                query.execute(&mut *tx).await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    /// Get all references from a symbol (what does this symbol call?)
    pub async fn get_outgoing_references(&self, symbol_id: i64) -> Result<Vec<SymbolReference>> {
        let refs = sqlx::query_as::<_, SymbolReference>(
            "SELECT id, source_symbol_id, target_name, target_symbol_id, kind, line FROM symbol_references WHERE source_symbol_id = ?"
        )
        .bind(symbol_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(refs)
    }


    /// Resolve target_symbol_id for references that point to known symbols.
    /// Uses multi-pass resolution: same file first, then global fallback.
    pub async fn resolve_references(&self) -> Result<u64> {
        let mut total = 0u64;

        // Pass 1: Same file — JOIN replaces correlated subquery
        let r1 = sqlx::query(
            r#"
            UPDATE symbol_references
            SET target_symbol_id = matched.target_id
            FROM (
                SELECT sr.id AS ref_id, MIN(s.id) AS target_id
                FROM symbol_references sr
                JOIN symbols src ON src.id = sr.source_symbol_id
                JOIN symbols s   ON s.name = sr.target_name
                                 AND s.file_id = src.file_id
                WHERE sr.target_symbol_id IS NULL
                GROUP BY sr.id
            ) AS matched
            WHERE symbol_references.id = matched.ref_id
            "#,
        )
        .execute(&self.pool)
        .await?;
        total += r1.rows_affected();

        // Pass 2: Same directory — CTE pre-computes dir prefix once per ref
        let r2 = sqlx::query(
            r#"
            WITH ref_dirs AS (
                SELECT sr.id AS ref_id, sr.target_name,
                       SUBSTR(sf.path, 1,
                              LENGTH(sf.path) - LENGTH(REPLACE(sf.path, '/', ''))
                       ) AS dir_prefix
                FROM symbol_references sr
                JOIN symbols src ON src.id = sr.source_symbol_id
                JOIN files sf    ON sf.id = src.file_id
                WHERE sr.target_symbol_id IS NULL
            )
            UPDATE symbol_references
            SET target_symbol_id = matched.target_id
            FROM (
                SELECT rd.ref_id, MIN(s.id) AS target_id
                FROM ref_dirs rd
                JOIN symbols s  ON s.name = rd.target_name
                JOIN files tf   ON tf.id = s.file_id
                                AND tf.path LIKE rd.dir_prefix || '%'
                GROUP BY rd.ref_id
            ) AS matched
            WHERE symbol_references.id = matched.ref_id
            "#,
        )
        .execute(&self.pool)
        .await?;
        total += r2.rows_affected();

        // Pass 3: Global fallback — any symbol with matching name
        let r3 = sqlx::query(
            r#"
            UPDATE symbol_references
            SET target_symbol_id = matched.target_id
            FROM (
                SELECT sr.id AS ref_id, MIN(s.id) AS target_id
                FROM symbol_references sr
                JOIN symbols s ON s.name = sr.target_name
                WHERE sr.target_symbol_id IS NULL
                GROUP BY sr.id
            ) AS matched
            WHERE symbol_references.id = matched.ref_id
            "#,
        )
        .execute(&self.pool)
        .await?;
        total += r3.rows_affected();

        Ok(total)
    }


    // === MCP Tool Support Methods ===

    /// Get symbols with optional filters
    pub async fn get_symbols(
        &self,
        file: Option<&str>,
        kind: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<SymbolWithPath>> {
        let mut query = String::from(
            r#"
            SELECT s.id, s.name, s.kind, s.line_start as line, s.line_end as end_line, s.signature, f.path as file_path
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE 1=1
            "#,
        );

        if file.is_some() {
            query.push_str(" AND f.path = ?");
        }
        if kind.is_some() {
            query.push_str(" AND s.kind = ?");
        }
        query.push_str(" ORDER BY f.path, s.line_start LIMIT ? OFFSET ?");

        let mut q = sqlx::query_as::<_, SymbolWithPath>(&query);

        if let Some(f) = file {
            q = q.bind(f);
        }
        if let Some(k) = kind {
            q = q.bind(k);
        }
        q = q.bind(limit).bind(offset);

        Ok(q.fetch_all(&self.pool).await?)
    }

    /// Get references by symbol name
    pub async fn get_references_by_name(
        &self,
        symbol_name: &str,
    ) -> Result<Vec<ReferenceWithPath>> {
        let refs = sqlx::query_as::<_, ReferenceWithPath>(
            r#"
            SELECT sr.id, sr.target_name, sr.kind as ref_kind, sr.line, f.path as file_path
            FROM symbol_references sr
            JOIN symbols s ON sr.source_symbol_id = s.id
            JOIN files f ON s.file_id = f.id
            WHERE sr.target_name = ?
            ORDER BY f.path, sr.line
            "#,
        )
        .bind(symbol_name)
        .fetch_all(&self.pool)
        .await?;

        Ok(refs)
    }


    // === Subproject / Monorepo Operations ===

    /// Upsert a subproject record.
    pub async fn upsert_subproject(
        &self,
        name: &str,
        path: &str,
        kind: &str,
        parent_path: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO subprojects (name, path, kind, parent_path)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                parent_path = excluded.parent_path
            RETURNING id
            "#,
        )
        .bind(name)
        .bind(path)
        .bind(kind)
        .bind(parent_path)
        .fetch_one(&self.pool)
        .await?;

        Ok(sqlx::Row::get(&result, "id"))
    }

    /// Assign a file to a subproject.
    pub async fn set_file_subproject(&self, file_id: i64, subproject_id: i64) -> Result<()> {
        sqlx::query("UPDATE files SET subproject_id = ? WHERE id = ?")
            .bind(subproject_id)
            .bind(file_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List all subprojects.
    pub async fn list_subprojects(&self) -> Result<Vec<SubprojectRecord>> {
        let rows = sqlx::query_as::<_, SubprojectRecord>(
            "SELECT id, name, path, kind, parent_path FROM subprojects ORDER BY path",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Clear all subproject records (before rescan).
    pub async fn clear_subprojects(&self) -> Result<()> {
        sqlx::query("UPDATE files SET subproject_id = NULL")
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM subprojects")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // === Index Metadata Operations ===

    /// Get a metadata value by key.
    pub async fn get_index_meta(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query_scalar::<_, String>("SELECT value FROM index_metadata WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Set a metadata key-value pair (upsert).
    pub async fn set_index_meta(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO index_metadata (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // === Chunk Embedding Cache ===

    /// Look up cached embeddings by content hashes. Returns a map of hash → embedding.
    /// Optimized with rkyv for zero-copy deserialization
    pub async fn get_cached_embeddings(
        &self,
        hashes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        if hashes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let mut result = std::collections::HashMap::new();
        // SQLite has a variable limit; batch in groups of 500
        for chunk in hashes.chunks(500) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query_str = format!(
                "SELECT content_hash, embedding FROM chunk_cache WHERE content_hash IN ({})",
                placeholders
            );
            let mut q = sqlx::query_as::<_, (String, Vec<u8>)>(&query_str);
            for h in chunk {
                q = q.bind(h);
            }
            let rows: Vec<(String, Vec<u8>)> = q.fetch_all(&self.pool).await?;
            for (hash, blob) in rows {
                // Try zero-copy deserialization with rkyv first
                if let Ok(archived) = rkyv::check_archived_root::<Vec<f32>>(&blob) {
                    // Zero-copy access - just read the archived data directly
                    let floats: Vec<f32> = archived.iter().copied().collect();
                    result.insert(hash, floats);
                } else {
                    // Fallback to legacy format (f32 little-endian bytes)
                    let floats: Vec<f32> = blob
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .collect();
                    result.insert(hash, floats);
                }
            }
        }
        Ok(result)
    }

    /// Clear the entire chunk embedding cache (used when embedding model changes).
    pub async fn clear_chunk_cache(&self) -> Result<()> {
        sqlx::query("DELETE FROM chunk_cache")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Reset indexing status of all files, forcing a full re-index.
    pub async fn reset_all_indexing_status(&self) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE files 
            SET indexing_status = 'pending',
                last_indexed_at = NULL
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store embeddings in the chunk cache.
    /// Now uses rkyv for better performance
    pub async fn store_cached_embeddings(&self, entries: &[(String, Vec<f32>)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        for chunk in entries.chunks(200) {
            let mut tx = self.pool.begin().await?;
            for (hash, embedding) in chunk {
                // Serialize with rkyv for zero-copy reads
                let blob = rkyv::to_bytes::<_, 256>(embedding)
                    .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

                sqlx::query(
                    "INSERT OR REPLACE INTO chunk_cache (content_hash, embedding) VALUES (?, ?)",
                )
                .bind(hash)
                .bind(blob.as_slice())
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }

    /// Get chunk cache statistics.
    pub async fn get_chunk_cache_stats(&self) -> Result<(i64, i64)> {
        // Returns (entry_count, total_size_bytes)
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chunk_cache")
            .fetch_one(&self.pool)
            .await?;

        // Approximate size: each entry ≈ 64 bytes hash + 1536 bytes embedding (384 * 4)
        let size_estimate = count.0 * (64 + 1536);

        Ok((count.0, size_estimate))
    }

    /// Evict oldest cache entries to maintain a maximum count.
    /// Uses LRU based on created_at timestamp.
    pub async fn evict_chunk_cache_to_limit(&self, max_entries: i64) -> Result<i64> {
        let (current_count, _) = self.get_chunk_cache_stats().await?;

        if current_count <= max_entries {
            return Ok(0);
        }

        let to_delete = current_count - max_entries;

        // Delete oldest entries (smallest created_at)
        let result = sqlx::query(
            r#"
            DELETE FROM chunk_cache 
            WHERE content_hash IN (
                SELECT content_hash FROM chunk_cache 
                ORDER BY created_at ASC 
                LIMIT ?
            )
            "#,
        )
        .bind(to_delete)
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected() as i64;
        if deleted > 0 {
            tracing::info!(
                "Cache eviction: removed {} old entries (limit: {})",
                deleted,
                max_entries
            );
        }

        Ok(deleted)
    }

    /// Evict cache entries older than specified days.
    pub async fn evict_chunk_cache_by_age(&self, max_age_days: i64) -> Result<i64> {
        let cutoff = chrono::Utc::now().timestamp() - (max_age_days * 24 * 60 * 60);

        let result = sqlx::query("DELETE FROM chunk_cache WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;

        let deleted = result.rows_affected() as i64;
        if deleted > 0 {
            tracing::info!(
                "Cache eviction: removed {} entries older than {} days",
                deleted,
                max_age_days
            );
        }

        Ok(deleted)
    }
}

// === Audit Log ===

impl SqliteStorage {
    /// Record a tool call in the audit log.
    pub async fn log_tool_call(
        &self,
        tool_name: &str,
        args_json: Option<&str>,
        latency_ms: u64,
        success: bool,
        error_msg: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO audit_log (tool_name, args_json, latency_ms, success, error_msg) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(tool_name)
        .bind(args_json)
        .bind(latency_ms as i64)
        .bind(success as i32)
        .bind(error_msg)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct SubprojectRecord {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub parent_path: Option<String>,
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_storage() -> (SqliteStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let storage = SqliteStorage::new(db_path.to_str().unwrap()).await.unwrap();
        storage.migrate().await.unwrap();
        (storage, temp_dir)
    }

    // -------------------------------------------------------------------------
    // Basic connectivity tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_storage_creation() {
        let (storage, _temp) = create_test_storage().await;
        assert!(storage.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn test_migration() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("migration_test.db");
        let storage = SqliteStorage::new(db_path.to_str().unwrap()).await.unwrap();

        // Migration should succeed
        let result = storage.migrate().await;
        assert!(result.is_ok());

        // Second migration should be idempotent
        let result = storage.migrate().await;
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // File operations tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_upsert_and_get_file() {
        let (storage, _temp) = create_test_storage().await;

        // Insert a file
        let file_id = storage
            .upsert_file("/test/file.rs", 12345, "abc123hash")
            .await
            .unwrap();
        assert!(file_id > 0);

        // Get the file back
        let file = storage.get_file("/test/file.rs").await.unwrap();
        assert!(file.is_some());
        let file = file.unwrap();
        assert_eq!(file.path, "/test/file.rs");
        assert_eq!(file.last_modified, 12345);
        assert_eq!(file.content_hash, "abc123hash");
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let (storage, _temp) = create_test_storage().await;

        // Insert first version
        let id1 = storage
            .upsert_file("/test/file.rs", 100, "hash1")
            .await
            .unwrap();

        // Update with new hash
        let id2 = storage
            .upsert_file("/test/file.rs", 200, "hash2")
            .await
            .unwrap();

        // Should return same ID
        assert_eq!(id1, id2);

        // File should be updated
        let file = storage.get_file("/test/file.rs").await.unwrap().unwrap();
        assert_eq!(file.last_modified, 200);
        assert_eq!(file.content_hash, "hash2");
    }

    #[tokio::test]
    async fn test_delete_file() {
        let (storage, _temp) = create_test_storage().await;

        storage
            .upsert_file("/test/to_delete.rs", 100, "hash")
            .await
            .unwrap();
        assert!(storage
            .get_file("/test/to_delete.rs")
            .await
            .unwrap()
            .is_some());

        storage.delete_file("/test/to_delete.rs").await.unwrap();
        assert!(storage
            .get_file("/test/to_delete.rs")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_get_file_count() {
        let (storage, _temp) = create_test_storage().await;

        assert_eq!(storage.get_file_count().await.unwrap(), 0);

        storage.upsert_file("/file1.rs", 100, "h1").await.unwrap();
        assert_eq!(storage.get_file_count().await.unwrap(), 1);

        storage.upsert_file("/file2.rs", 100, "h2").await.unwrap();
        assert_eq!(storage.get_file_count().await.unwrap(), 2);

        storage.delete_file("/file1.rs").await.unwrap();
        assert_eq!(storage.get_file_count().await.unwrap(), 1);
    }

    // -------------------------------------------------------------------------
    // Symbol operations tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_insert_and_get_symbols() {
        let (storage, _temp) = create_test_storage().await;

        let file_id = storage.upsert_file("/test.rs", 100, "hash").await.unwrap();

        let symbols = vec![
            Symbol {
                id: 0,
                file_id,
                name: "my_function".to_string(),
                kind: crate::models::chunk::SymbolKind::Function,
                line_start: 10,
                line_end: 20,
                signature: Some("fn my_function()".to_string()),
            },
            Symbol {
                id: 0,
                file_id,
                name: "MyStruct".to_string(),
                kind: crate::models::chunk::SymbolKind::Struct,
                line_start: 30,
                line_end: 40,
                signature: Some("struct MyStruct".to_string()),
            },
        ];

        storage.insert_symbols(file_id, &symbols).await.unwrap();

        let retrieved = storage.get_file_symbols(file_id).await.unwrap();
        assert_eq!(retrieved.len(), 2);
        assert!(retrieved.iter().any(|s| s.name == "my_function"));
        assert!(retrieved.iter().any(|s| s.name == "MyStruct"));
    }

    #[tokio::test]
    async fn test_get_symbol_by_name() {
        let (storage, _temp) = create_test_storage().await;

        let file_id = storage.upsert_file("/test.rs", 100, "hash").await.unwrap();
        let symbols = vec![Symbol {
            id: 0,
            file_id,
            name: "unique_symbol".to_string(),
            kind: crate::models::chunk::SymbolKind::Function,
            line_start: 1,
            line_end: 5,
            signature: None,
        }];
        storage.insert_symbols(file_id, &symbols).await.unwrap();

        let found = storage.get_symbol_by_name("unique_symbol").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "unique_symbol");

        let not_found = storage.get_symbol_by_name("nonexistent").await.unwrap();
        assert!(not_found.is_empty());
    }

    #[tokio::test]
    async fn test_get_symbols_pagination() {
        let (storage, _temp) = create_test_storage().await;

        let file_id = storage.upsert_file("/test.rs", 100, "hash").await.unwrap();

        // Insert 10 symbols
        let mut symbols = vec![];
        for i in 0..10 {
            symbols.push(Symbol {
                id: 0,
                file_id,
                name: format!("symbol_{}", i),
                kind: crate::models::chunk::SymbolKind::Function,
                line_start: i,
                line_end: i + 1,
                signature: None,
            });
        }
        storage.insert_symbols(file_id, &symbols).await.unwrap();

        // Test pagination
        let page1 = storage.get_symbols(None, None, 0, 5).await.unwrap();
        assert_eq!(page1.len(), 5);

        let page2 = storage.get_symbols(None, None, 5, 5).await.unwrap();
        assert_eq!(page2.len(), 5);

        // Different names between pages
        let names1: Vec<_> = page1.iter().map(|s| &s.name).collect();
        let names2: Vec<_> = page2.iter().map(|s| &s.name).collect();
        for name in &names1 {
            assert!(!names2.contains(name));
        }
    }

    // -------------------------------------------------------------------------
    // Embedding cache tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_embedding_cache() {
        let (storage, _temp) = create_test_storage().await;

        let entries = vec![
            ("hash1".to_string(), vec![0.1f32, 0.2, 0.3]),
            ("hash2".to_string(), vec![0.4f32, 0.5, 0.6]),
        ];

        // Store embeddings
        storage.store_cached_embeddings(&entries).await.unwrap();

        // Retrieve
        let cached = storage
            .get_cached_embeddings(&[
                "hash1".to_string(),
                "hash2".to_string(),
                "hash3".to_string(),
            ])
            .await
            .unwrap();

        assert_eq!(cached.len(), 2);
        assert!(cached.contains_key("hash1"));
        assert!(cached.contains_key("hash2"));
        assert!(!cached.contains_key("hash3")); // not stored

        // Verify values
        let emb1 = cached.get("hash1").unwrap();
        assert_eq!(emb1.len(), 3);
        assert!((emb1[0] - 0.1).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_clear_chunk_cache() {
        let (storage, _temp) = create_test_storage().await;

        let entries = vec![("hash1".to_string(), vec![0.1f32])];
        storage.store_cached_embeddings(&entries).await.unwrap();

        let cached = storage
            .get_cached_embeddings(&["hash1".to_string()])
            .await
            .unwrap();
        assert_eq!(cached.len(), 1);

        storage.clear_chunk_cache().await.unwrap();

        let cached = storage
            .get_cached_embeddings(&["hash1".to_string()])
            .await
            .unwrap();
        assert_eq!(cached.len(), 0);
    }

    // -------------------------------------------------------------------------
    // Index meta tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_index_meta() {
        let (storage, _temp) = create_test_storage().await;

        // Initially empty
        let value = storage.get_index_meta("test_key").await.unwrap();
        assert!(value.is_none());

        // Set value
        storage
            .set_index_meta("test_key", "test_value")
            .await
            .unwrap();

        // Get value
        let value = storage.get_index_meta("test_key").await.unwrap();
        assert_eq!(value, Some("test_value".to_string()));

        // Update value
        storage
            .set_index_meta("test_key", "new_value")
            .await
            .unwrap();
        let value = storage.get_index_meta("test_key").await.unwrap();
        assert_eq!(value, Some("new_value".to_string()));
    }




    // -------------------------------------------------------------------------
    // Audit log tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_audit_log() {
        let (storage, _temp) = create_test_storage().await;

        storage
            .log_tool_call("search", Some(r#"{"query":"test"}"#), 150, true, None)
            .await
            .unwrap();
        storage
            .log_tool_call("get_symbols", None, 50, false, Some("error occurred"))
            .await
            .unwrap();

        // Just verify no errors - audit log reading would need additional method
    }
}
