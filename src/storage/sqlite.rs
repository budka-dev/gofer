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
    ActiveError, ApiEndpointInfo, ConfigKey, CrossStackLink, Dependency, DependencyUsage,
    DependencyUsageInfo, FrontendApiCallInfo, IndexedFile, ReferenceWithPath, Rule, Symbol,
    SymbolReference, SymbolWithPath, TypeFingerprint, VueTree,
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

/// Detects SQLITE_BUSY (code 5) and SQLITE_LOCKED (code 6) — the two errors that
/// indicate a transient contention failure that's safe to retry.
fn is_busy_error(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        if let Some(code) = db_err.code() {
            return code == "5" || code == "6";
        }
    }
    false
}

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

    pub fn get_stats(&self) -> (u64, u64, u64) {
        (
            self.total_queries.load(Ordering::Relaxed),
            self.slow_queries.load(Ordering::Relaxed),
            self.total_query_time_ms.load(Ordering::Relaxed),
        )
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

    /// Execute a write operation that may transiently fail with SQLITE_BUSY (code 5)
    /// when another connection holds the write lock longer than busy_timeout. Retries
    /// with exponential back-off (50ms → 200ms → 1s → 5s) before bubbling up.
    pub async fn with_write_retry<F, Fut, T>(&self, mut op: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
    {
        const BACKOFF_MS: &[u64] = &[50, 200, 1000, 5000];
        let mut attempt = 0;
        loop {
            match op().await {
                Ok(v) => return Ok(v),
                Err(e) if is_busy_error(&e) && attempt < BACKOFF_MS.len() => {
                    tracing::warn!(
                        "SQLite busy on write (attempt {}), retrying in {}ms",
                        attempt + 1,
                        BACKOFF_MS[attempt]
                    );
                    tokio::time::sleep(Duration::from_millis(BACKOFF_MS[attempt])).await;
                    attempt += 1;
                }
                Err(e) => return Err(StorageError::Database(e)),
            }
        }
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

    /// Get symbol by id
    pub async fn get_symbol_by_id(&self, id: i64) -> Result<Option<Symbol>> {
        let symbol = sqlx::query_as::<_, Symbol>(
            "SELECT id, file_id, name, kind, line_start, line_end, signature FROM symbols WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(symbol)
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

    /// Find symbol at specific line in file
    pub async fn find_symbol_at_line(&self, file_path: &str, line: i32) -> Result<Option<Symbol>> {
        let symbol = sqlx::query_as::<_, Symbol>(
            r#"
            SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature 
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE f.path = ? AND s.line_start <= ? AND s.line_end >= ?
            ORDER BY (s.line_end - s.line_start) ASC
            LIMIT 1
            "#,
        )
        .bind(file_path)
        .bind(line)
        .bind(line)
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

    /// Get all references to a symbol (who calls this symbol?)
    pub async fn get_incoming_references(&self, symbol_name: &str) -> Result<Vec<SymbolReference>> {
        let refs = sqlx::query_as::<_, SymbolReference>(
            "SELECT id, source_symbol_id, target_name, target_symbol_id, kind, line FROM symbol_references WHERE target_name = ?"
        )
        .bind(symbol_name)
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

    // === Dependency Operations ===

    /// Upsert a dependency
    pub async fn upsert_dependency(&self, dep: &Dependency) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO dependencies (name, version, ecosystem, features, dev_only, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(name, ecosystem) DO UPDATE SET
                version = excluded.version,
                features = excluded.features,
                dev_only = excluded.dev_only,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&dep.name)
        .bind(&dep.version)
        .bind(&dep.ecosystem)
        .bind(&dep.features)
        .bind(dep.dev_only)
        .bind(dep.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all dependencies
    pub async fn get_dependencies(&self) -> Result<Vec<Dependency>> {
        let deps = sqlx::query_as::<_, Dependency>(
            "SELECT id, name, version, ecosystem, features, dev_only, updated_at FROM dependencies ORDER BY ecosystem, name"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(deps)
    }

    /// Get dependencies by ecosystem
    pub async fn get_dependencies_by_ecosystem(&self, ecosystem: &str) -> Result<Vec<Dependency>> {
        let deps = sqlx::query_as::<_, Dependency>(
            "SELECT id, name, version, ecosystem, features, dev_only, updated_at FROM dependencies WHERE ecosystem = ?"
        )
        .bind(ecosystem)
        .fetch_all(&self.pool)
        .await?;

        Ok(deps)
    }

    // === Rules Operations ===

    /// Insert or replace rules (clears existing rules from source, batched in transaction)
    pub async fn upsert_rules(&self, rules: &[Rule], source: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM rules WHERE source = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;

        for rule in rules {
            sqlx::query("INSERT INTO rules (category, rule, priority, source) VALUES (?, ?, ?, ?)")
                .bind(&rule.category)
                .bind(&rule.rule)
                .bind(rule.priority)
                .bind(&rule.source)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Get all rules
    pub async fn get_rules(&self) -> Result<Vec<Rule>> {
        let rules = sqlx::query_as::<_, Rule>(
            "SELECT id, category, rule, priority, source FROM rules ORDER BY priority DESC, category"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rules)
    }

    /// Mark a file as golden sample
    pub async fn mark_golden_sample(
        &self,
        file_id: i64,
        category: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO golden_samples (file_id, category, description)
            VALUES (?, ?, ?)
            ON CONFLICT(file_id) DO UPDATE SET
                category = excluded.category,
                description = excluded.description
            "#,
        )
        .bind(file_id)
        .bind(category)
        .bind(description)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get golden sample file paths
    pub async fn get_golden_samples(&self) -> Result<Vec<(String, Option<String>)>> {
        let samples = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT f.path, g.category FROM golden_samples g JOIN files f ON g.file_id = f.id",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(samples)
    }

    // === Dependency Usage Operations ===

    /// Record dependency usage for a file
    pub async fn record_dependency_usage(
        &self,
        file_id: i64,
        dep_name: &str,
        ecosystem: &str,
        line: i32,
        usage_type: &str,
        import_path: &str,
        items: Option<&str>,
    ) -> Result<()> {
        // Find or create dependency
        let dep_id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM dependencies WHERE name = ? AND ecosystem = ?")
                .bind(dep_name)
                .bind(ecosystem)
                .fetch_optional(&self.pool)
                .await?;

        let dep_id = match dep_id {
            Some(id) => id,
            None => {
                // Create unknown dependency placeholder
                sqlx::query(
                    "INSERT OR IGNORE INTO dependencies (name, version, ecosystem, updated_at) VALUES (?, '?', ?, ?)"
                )
                .bind(dep_name)
                .bind(ecosystem)
                .bind(chrono::Utc::now().timestamp())
                .execute(&self.pool)
                .await?;

                sqlx::query_scalar("SELECT id FROM dependencies WHERE name = ? AND ecosystem = ?")
                    .bind(dep_name)
                    .bind(ecosystem)
                    .fetch_one(&self.pool)
                    .await?
            }
        };

        sqlx::query(
            r#"
            INSERT INTO dependency_usage (dependency_id, file_id, line, usage_type, import_path, items)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(file_id, line, import_path) DO UPDATE SET
                dependency_id = excluded.dependency_id,
                usage_type = excluded.usage_type,
                items = excluded.items
            "#
        )
        .bind(dep_id)
        .bind(file_id)
        .bind(line)
        .bind(usage_type)
        .bind(import_path)
        .bind(items)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Clear dependency usages for a file
    pub async fn clear_dependency_usage(&self, file_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM dependency_usage WHERE file_id = ?")
            .bind(file_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get all usages of a specific dependency
    pub async fn get_dependency_usages(&self, dep_name: &str) -> Result<Vec<DependencyUsage>> {
        let usages = sqlx::query_as::<_, DependencyUsage>(
            r#"
            SELECT du.id, du.dependency_id, du.file_id, du.line, du.usage_type, du.import_path, du.items
            FROM dependency_usage du
            JOIN dependencies d ON du.dependency_id = d.id
            WHERE d.name = ?
            ORDER BY du.file_id, du.line
            "#
        )
        .bind(dep_name)
        .fetch_all(&self.pool)
        .await?;

        Ok(usages)
    }

    /// Get impact analysis for a dependency (all files that use it)
    pub async fn get_dependency_impact(
        &self,
        dep_name: &str,
    ) -> Result<Vec<(String, i32, String)>> {
        let impact = sqlx::query_as::<_, (String, i32, String)>(
            r#"
            SELECT f.path, du.line, du.usage_type
            FROM dependency_usage du
            JOIN dependencies d ON du.dependency_id = d.id
            JOIN files f ON du.file_id = f.id
            WHERE d.name = ?
            ORDER BY f.path, du.line
            "#,
        )
        .bind(dep_name)
        .fetch_all(&self.pool)
        .await?;

        Ok(impact)
    }

    /// Get all dependencies used by a file
    pub async fn get_file_dependencies(&self, file_id: i64) -> Result<Vec<(String, String, i32)>> {
        let deps = sqlx::query_as::<_, (String, String, i32)>(
            r#"
            SELECT d.name, d.version, du.line
            FROM dependency_usage du
            JOIN dependencies d ON du.dependency_id = d.id
            WHERE du.file_id = ?
            ORDER BY du.line
            "#,
        )
        .bind(file_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(deps)
    }

    /// Get unused dependencies (no usages recorded)
    pub async fn get_unused_dependencies(&self) -> Result<Vec<Dependency>> {
        let deps = sqlx::query_as::<_, Dependency>(
            r#"
            SELECT d.id, d.name, d.version, d.ecosystem, d.features, d.dev_only, d.updated_at
            FROM dependencies d
            WHERE NOT EXISTS (SELECT 1 FROM dependency_usage du WHERE du.dependency_id = d.id)
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(deps)
    }

    // === Diagnostics Operations ===

    /// Clear all active errors
    pub async fn clear_active_errors(&self) -> Result<()> {
        sqlx::query("DELETE FROM active_errors")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clear active errors for a specific file
    pub async fn clear_file_errors(&self, file_path: &str) -> Result<()> {
        sqlx::query("DELETE FROM active_errors WHERE file_path = ?")
            .bind(file_path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert an active error
    pub async fn insert_error(
        &self,
        file_path: &str,
        line: i32,
        column: Option<i32>,
        severity: &str,
        code: Option<&str>,
        message: &str,
        suggestion: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO active_errors (file_path, line, column, severity, code, message, suggestion, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(file_path, line, message) DO UPDATE SET
                column = excluded.column,
                severity = excluded.severity,
                code = excluded.code,
                suggestion = excluded.suggestion,
                updated_at = excluded.updated_at
            "#
        )
        .bind(file_path)
        .bind(line)
        .bind(column)
        .bind(severity)
        .bind(code)
        .bind(message)
        .bind(suggestion)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all active errors
    pub async fn get_active_errors(&self) -> Result<Vec<ActiveError>> {
        let errors = sqlx::query_as::<_, ActiveError>(
            "SELECT id, file_path, line, column, severity, code, message, suggestion, updated_at FROM active_errors ORDER BY file_path, line"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(errors)
    }

    /// Get active errors for a specific file
    pub async fn get_file_errors(&self, file_path: &str) -> Result<Vec<ActiveError>> {
        let errors = sqlx::query_as::<_, ActiveError>(
            "SELECT id, file_path, line, column, severity, code, message, suggestion, updated_at FROM active_errors WHERE file_path = ? ORDER BY line"
        )
        .bind(file_path)
        .fetch_all(&self.pool)
        .await?;

        Ok(errors)
    }

    /// Count errors by severity
    pub async fn count_errors(&self) -> Result<(i64, i64)> {
        let errors: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM active_errors WHERE severity = 'error'")
                .fetch_one(&self.pool)
                .await?;
        let warnings: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM active_errors WHERE severity = 'warning'")
                .fetch_one(&self.pool)
                .await?;
        Ok((errors, warnings))
    }

    // === Config Keys Operations ===

    /// Upsert a config key
    pub async fn upsert_config_key(
        &self,
        key_name: &str,
        data_type: Option<&str>,
        source: &str,
        description: Option<&str>,
        default_value: Option<&str>,
        required: bool,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO config_keys (key_name, data_type, source, description, default_value, required)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(key_name) DO UPDATE SET
                data_type = COALESCE(excluded.data_type, config_keys.data_type),
                source = excluded.source,
                description = COALESCE(excluded.description, config_keys.description),
                default_value = COALESCE(excluded.default_value, config_keys.default_value),
                required = excluded.required
            "#
        )
        .bind(key_name)
        .bind(data_type)
        .bind(source)
        .bind(description)
        .bind(default_value)
        .bind(if required { 1 } else { 0 })
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all config keys
    pub async fn get_config_keys(&self) -> Result<Vec<ConfigKey>> {
        let keys = sqlx::query_as::<_, ConfigKey>(
            "SELECT id, key_name, data_type, source, description, default_value, required FROM config_keys ORDER BY key_name"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(keys)
    }

    // === Vue Tree Operations ===

    /// Upsert Vue component tree
    pub async fn upsert_vue_tree(
        &self,
        file_id: i64,
        tree_text: &str,
        components: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO vue_trees (file_id, tree_text, components, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(file_id) DO UPDATE SET
                tree_text = excluded.tree_text,
                components = excluded.components,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(file_id)
        .bind(tree_text)
        .bind(components)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get Vue tree for a file
    pub async fn get_vue_tree(&self, file_path: &str) -> Result<Option<VueTree>> {
        let tree = sqlx::query_as::<_, VueTree>(
            r#"
            SELECT vt.id, vt.file_id, vt.tree_text, vt.components, vt.updated_at
            FROM vue_trees vt
            JOIN files f ON vt.file_id = f.id
            WHERE f.path = ?
            "#,
        )
        .bind(file_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(tree)
    }

    // === Domain Operations ===

    /// Update file domain
    pub async fn update_file_domain(
        &self,
        file_id: i64,
        domain: &str,
        tech_stack: &[String],
    ) -> Result<()> {
        // Serialize with rkyv for BLOB column
        let tech_vec = tech_stack.to_vec();
        let tech_blob = rkyv::to_bytes::<_, 256>(&tech_vec)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

        // Also keep JSON for backward compatibility during migration
        let tech_json = serde_json::to_string(tech_stack).unwrap_or_default();

        sqlx::query(
            "UPDATE files SET domain = ?, tech_stack = ?, tech_stack_blob = ? WHERE id = ?",
        )
        .bind(domain)
        .bind(&tech_json)
        .bind(tech_blob.as_slice())
        .bind(file_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert entity link
    pub async fn insert_entity_link(
        &self,
        backend_id: i64,
        frontend_id: i64,
        confidence: f64,
        link_type: &str,
        matched_fields: &[String],
    ) -> Result<()> {
        // Serialize with rkyv for BLOB column
        let fields_vec = matched_fields.to_vec();
        let fields_blob = rkyv::to_bytes::<_, 256>(&fields_vec)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

        // Also keep JSON for backward compatibility
        let fields_json = serde_json::to_string(matched_fields).unwrap_or_default();

        sqlx::query(
            r#"
            INSERT INTO entity_links (backend_symbol_id, frontend_symbol_id, confidence, link_type, matched_fields, matched_fields_blob)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(backend_symbol_id, frontend_symbol_id) DO UPDATE SET
                confidence = excluded.confidence,
                link_type = excluded.link_type,
                matched_fields = excluded.matched_fields,
                matched_fields_blob = excluded.matched_fields_blob
            "#
        )
        .bind(backend_id)
        .bind(frontend_id)
        .bind(confidence)
        .bind(link_type)
        .bind(&fields_json)
        .bind(fields_blob.as_slice())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert API endpoint
    pub async fn insert_api_endpoint(
        &self,
        method: &str,
        path: &str,
        file_id: i64,
        line: i32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO api_endpoints (method, path, file_id, line)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(method, path) DO UPDATE SET
                file_id = excluded.file_id,
                line = excluded.line
            "#,
        )
        .bind(method)
        .bind(path)
        .bind(file_id)
        .bind(line)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert frontend API call
    pub async fn insert_frontend_api_call(
        &self,
        method: Option<&str>,
        path: &str,
        path_pattern: &str,
        file_id: i64,
        line: i32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO frontend_api_calls (method, path, path_pattern, file_id, line) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(method)
        .bind(path)
        .bind(path_pattern)
        .bind(file_id)
        .bind(line)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get linked frontend symbol for a backend symbol
    pub async fn get_frontend_links(
        &self,
        backend_symbol_id: i64,
    ) -> Result<Vec<(i64, String, f64)>> {
        let links = sqlx::query_as::<_, (i64, String, f64)>(
            r#"
            SELECT el.frontend_symbol_id, s.name, el.confidence
            FROM entity_links el
            JOIN symbols s ON el.frontend_symbol_id = s.id
            WHERE el.backend_symbol_id = ?
            ORDER BY el.confidence DESC
            "#,
        )
        .bind(backend_symbol_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(links)
    }

    /// Get linked backend symbol for a frontend symbol
    pub async fn get_backend_links(
        &self,
        frontend_symbol_id: i64,
    ) -> Result<Vec<(i64, String, f64)>> {
        let links = sqlx::query_as::<_, (i64, String, f64)>(
            r#"
            SELECT el.backend_symbol_id, s.name, el.confidence
            FROM entity_links el
            JOIN symbols s ON el.backend_symbol_id = s.id
            WHERE el.frontend_symbol_id = ?
            ORDER BY el.confidence DESC
            "#,
        )
        .bind(frontend_symbol_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(links)
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

    /// Get dependencies with optional ecosystem filter
    pub async fn get_dependencies_filtered(
        &self,
        ecosystem: Option<&str>,
    ) -> Result<Vec<Dependency>> {
        if let Some(eco) = ecosystem {
            self.get_dependencies_by_ecosystem(eco).await
        } else {
            self.get_dependencies().await
        }
    }

    /// Get dependency usage info for a specific dependency
    pub async fn get_dependency_usage(&self, dep_name: &str) -> Result<Vec<DependencyUsageInfo>> {
        let usages = sqlx::query_as::<_, DependencyUsageInfo>(
            r#"
            SELECT du.id, du.line, du.usage_type, du.import_path, du.items, f.path as file_path
            FROM dependency_usage du
            JOIN dependencies d ON du.dependency_id = d.id
            JOIN files f ON du.file_id = f.id
            WHERE d.name = ?
            ORDER BY f.path, du.line
            "#,
        )
        .bind(dep_name)
        .fetch_all(&self.pool)
        .await?;

        Ok(usages)
    }

    /// Get errors with optional filters
    pub async fn get_errors(
        &self,
        file: Option<&str>,
        severity: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ActiveError>> {
        let mut query = String::from(
            "SELECT id, file_path, line, column, severity, code, message, suggestion, updated_at FROM active_errors WHERE 1=1"
        );

        if file.is_some() {
            query.push_str(" AND file_path = ?");
        }
        if severity.is_some() {
            query.push_str(" AND severity = ?");
        }
        query.push_str(" ORDER BY file_path, line LIMIT ? OFFSET ?");

        let mut q = sqlx::query_as::<_, ActiveError>(&query);

        if let Some(f) = file {
            q = q.bind(f);
        }
        if let Some(s) = severity {
            q = q.bind(s);
        }
        q = q.bind(limit).bind(offset);

        Ok(q.fetch_all(&self.pool).await?)
    }

    /// Get domain statistics
    pub async fn get_domain_stats(&self) -> Result<Vec<(String, i64)>> {
        let stats = sqlx::query_as::<_, (String, i64)>(
            r#"
            SELECT COALESCE(domain, 'unknown') as domain, COUNT(*) as count
            FROM files
            GROUP BY domain
            ORDER BY count DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(stats)
    }

    /// Get all API endpoints
    pub async fn get_api_endpoints(&self) -> Result<Vec<ApiEndpointInfo>> {
        let endpoints = sqlx::query_as::<_, ApiEndpointInfo>(
            r#"
            SELECT id, method, path, file_id, line
            FROM api_endpoints
            ORDER BY path, method
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(endpoints)
    }

    /// Get all frontend API calls
    pub async fn get_frontend_api_calls(&self) -> Result<Vec<FrontendApiCallInfo>> {
        let calls = sqlx::query_as::<_, FrontendApiCallInfo>(
            r#"
            SELECT id, method, path, path_pattern, file_id, line
            FROM frontend_api_calls
            ORDER BY path
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(calls)
    }

    // === Type Fingerprints Operations ===

    /// Upsert type fingerprint
    pub async fn upsert_type_fingerprint(
        &self,
        file_id: i64,
        symbol_id: i64,
        type_name: &str,
        language: &str,
        fields_json: &str,
        fields_normalized: &str,
        field_count: i32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO type_fingerprints (file_id, symbol_id, type_name, language, fields_json, fields_normalized, field_count)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(symbol_id) DO UPDATE SET
                type_name = excluded.type_name,
                language = excluded.language,
                fields_json = excluded.fields_json,
                fields_normalized = excluded.fields_normalized,
                field_count = excluded.field_count
            "#
        )
        .bind(file_id)
        .bind(symbol_id)
        .bind(type_name)
        .bind(language)
        .bind(fields_json)
        .bind(fields_normalized)
        .bind(field_count)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Получить все fingerprints для языка
    pub async fn get_fingerprints_by_language(
        &self,
        language: &str,
    ) -> Result<Vec<TypeFingerprint>> {
        let fps = sqlx::query_as::<_, TypeFingerprint>(
            r#"
            SELECT tf.id, tf.file_id, tf.symbol_id, tf.type_name, tf.language,
                   tf.fields_json, tf.fields_normalized, tf.field_count,
                   f.path as file_path
            FROM type_fingerprints tf
            JOIN files f ON tf.file_id = f.id
            WHERE tf.language = ? AND tf.field_count >= 3
            ORDER BY tf.type_name
            "#,
        )
        .bind(language)
        .fetch_all(&self.pool)
        .await?;

        Ok(fps)
    }

    /// Очистить fingerprints для файла
    pub async fn clear_file_fingerprints(&self, file_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM type_fingerprints WHERE file_id = ?")
            .bind(file_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // === Cross-Stack Links Operations ===

    /// Upsert cross-stack link
    pub async fn upsert_cross_stack_link(
        &self,
        source_file: &str,
        target_file: &str,
        source_symbol: &str,
        target_symbol: &str,
        link_type: &str,
        weight: f64,
        metadata: &str,
    ) -> Result<()> {
        // Serialize metadata with rkyv for BLOB column
        let metadata_string = metadata.to_string();
        let metadata_blob = rkyv::to_bytes::<_, 256>(&metadata_string)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

        sqlx::query(
            r#"
            INSERT INTO cross_stack_links (source_file, target_file, source_symbol, target_symbol, link_type, weight, metadata, metadata_blob)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_symbol, target_symbol, link_type) DO UPDATE SET
                source_file = excluded.source_file,
                target_file = excluded.target_file,
                weight = excluded.weight,
                metadata = excluded.metadata,
                metadata_blob = excluded.metadata_blob,
                created_at = strftime('%s', 'now')
            "#
        )
        .bind(source_file)
        .bind(target_file)
        .bind(source_symbol)
        .bind(target_symbol)
        .bind(link_type)
        .bind(weight)
        .bind(metadata)
        .bind(metadata_blob.as_slice())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Получить все cross-stack связи для файла
    pub async fn get_cross_stack_links_for_file(
        &self,
        file_path: &str,
    ) -> Result<Vec<CrossStackLink>> {
        let links = sqlx::query_as::<_, CrossStackLink>(
            r#"
            SELECT id, source_file, target_file, source_symbol, target_symbol, link_type, weight, 
                   COALESCE(metadata, '') as metadata, created_at
            FROM cross_stack_links
            WHERE source_file = ? OR target_file = ?
            ORDER BY weight DESC
            "#,
        )
        .bind(file_path)
        .bind(file_path)
        .fetch_all(&self.pool)
        .await?;

        Ok(links)
    }

    /// Получить все cross-stack связи определённого типа
    pub async fn get_cross_stack_links_by_type(
        &self,
        link_type: &str,
    ) -> Result<Vec<CrossStackLink>> {
        let links = sqlx::query_as::<_, CrossStackLink>(
            r#"
            SELECT id, source_file, target_file, source_symbol, target_symbol, link_type, weight, 
                   COALESCE(metadata, '') as metadata, created_at
            FROM cross_stack_links
            WHERE link_type = ?
            ORDER BY weight DESC
            "#,
        )
        .bind(link_type)
        .fetch_all(&self.pool)
        .await?;

        Ok(links)
    }

    /// Очистить structural links (перед пересчётом)
    pub async fn clear_structural_links(&self) -> Result<()> {
        sqlx::query("DELETE FROM cross_stack_links WHERE link_type = 'structural'")
            .execute(&self.pool)
            .await?;
        Ok(())
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
    // Rules tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_rules() {
        let (storage, _temp) = create_test_storage().await;

        // Get rules (may be empty initially)
        let rules = storage.get_rules().await.unwrap();
        // Just verify no error - rules list exists (may be empty)
        assert!(rules.is_empty() || !rules.is_empty());
    }

    // -------------------------------------------------------------------------
    // Error handling tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_active_errors() {
        let (storage, _temp) = create_test_storage().await;

        // Get errors (may be empty initially)
        let errors = storage.get_active_errors().await.unwrap();
        assert!(errors.is_empty());

        // Clear errors should work even if empty
        storage.clear_active_errors().await.unwrap();
    }

    // -------------------------------------------------------------------------
    // Golden samples tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_golden_samples() {
        let (storage, _temp) = create_test_storage().await;

        // Get samples (may be empty initially)
        let samples = storage.get_golden_samples().await.unwrap();
        assert!(samples.is_empty());
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
