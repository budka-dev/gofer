use std::sync::Arc;
use tokio::sync::RwLock;

use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator,
    StringArray, UInt32Array,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::{
    connect,
    index::Index,
    query::{ExecutableQuery, QueryBase},
    table::OptimizeAction,
    Connection, DistanceType, Table,
};
use thiserror::Error;

use crate::models::CodeChunk;

#[derive(Error, Debug)]
pub enum LanceError {
    #[error("LanceDB error: {0}")]
    Lance(#[from] lancedb::Error),
    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, LanceError>;

/// Escape a string value for use in a DataFusion SQL filter expression.
/// Doubles single quotes and strips null bytes to prevent injection.
fn escape_filter_string(s: &str) -> String {
    s.replace('\0', "").replace('\'', "''")
}

const TABLE_NAME: &str = "code_chunks";

/// LanceDB storage for code chunk vectors
pub struct LanceStorage {
    db: Connection,
    table: RwLock<Option<Table>>,
    vector_dim: i32,
}

impl LanceStorage {
    /// Создать LanceDB storage с указанной размерностью вектора.
    pub async fn new(db_path: &str, vector_dim: usize) -> Result<Self> {
        // std::fs::create_dir_all(db_path)?; // Removed as per instruction

        let db = connect(db_path).execute().await?;

        let storage = Self {
            db,
            table: RwLock::new(None),
            vector_dim: vector_dim as i32,
        };
        storage.ensure_table().await?;

        Ok(storage)
    }

    /// Get the schema for code chunks table
    fn schema(&self) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("file_path", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("line_start", DataType::UInt32, false),
            Field::new("line_end", DataType::UInt32, false),
            Field::new("symbol_name", DataType::Utf8, true),
            Field::new("symbol_kind", DataType::Utf8, true),
            Field::new("symbol_path", DataType::Utf8, true),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.vector_dim,
                ),
                false,
            ),
        ]))
    }

    /// Ensure table exists
    async fn ensure_table(&self) -> Result<()> {
        let table_names = self.db.table_names().execute().await?;

        if table_names.contains(&TABLE_NAME.to_string()) {
            let mut table_guard = self.table.write().await;
            if table_guard.is_none() {
                *table_guard = Some(self.db.open_table(TABLE_NAME).execute().await?);
            }
        }

        Ok(())
    }

    /// Insert or update code chunks with their embeddings
    pub async fn upsert_chunks(&self, chunks: &[CodeChunk], embeddings: &[Vec<f32>]) -> Result<()> {
        if chunks.is_empty() || embeddings.is_empty() {
            return Ok(());
        }

        let schema = self.schema();

        // Build arrays
        let ids: ArrayRef = Arc::new(StringArray::from(
            chunks.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ));
        let file_paths: ArrayRef = Arc::new(StringArray::from(
            chunks
                .iter()
                .map(|c| c.file_path.as_str())
                .collect::<Vec<_>>(),
        ));
        let contents: ArrayRef = Arc::new(StringArray::from(
            chunks
                .iter()
                .map(|c| c.content.as_str())
                .collect::<Vec<_>>(),
        ));
        let line_starts: ArrayRef = Arc::new(UInt32Array::from(
            chunks.iter().map(|c| c.line_start).collect::<Vec<_>>(),
        ));
        let line_ends: ArrayRef = Arc::new(UInt32Array::from(
            chunks.iter().map(|c| c.line_end).collect::<Vec<_>>(),
        ));
        let symbol_names: ArrayRef = Arc::new(StringArray::from(
            chunks
                .iter()
                .map(|c| c.symbol_name.as_deref())
                .collect::<Vec<_>>(),
        ));
        let symbol_kinds: ArrayRef = Arc::new(StringArray::from(
            chunks
                .iter()
                .map(|c| c.symbol_kind.as_ref().map(|k| k.as_str()))
                .collect::<Vec<_>>(),
        ));
        let symbol_paths: ArrayRef = Arc::new(StringArray::from(
            chunks
                .iter()
                .map(|c| c.symbol_path.as_deref())
                .collect::<Vec<_>>(),
        ));

        // Build vector array
        let flat_vectors: Vec<f32> = embeddings.iter().flatten().copied().collect();
        let values_array = Float32Array::from(flat_vectors);
        let field = Arc::new(Field::new("item", DataType::Float32, true));
        let vectors: ArrayRef = Arc::new(FixedSizeListArray::new(
            field,
            self.vector_dim,
            Arc::new(values_array),
            None,
        ));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                ids,
                file_paths,
                contents,
                line_starts,
                line_ends,
                symbol_names,
                symbol_kinds,
                symbol_paths,
                vectors,
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());
        let table_opt = (*self.table.read().await).clone();

        if let Some(table) = table_opt {
            // Batch delete: build single filter for all unique file paths
            let file_paths: Vec<_> = chunks.iter().map(|c| c.file_path.as_str()).collect();
            let unique_paths: std::collections::HashSet<_> = file_paths.into_iter().collect();

            if !unique_paths.is_empty() {
                let conditions: Vec<String> = unique_paths
                    .iter()
                    .map(|p| format!("file_path = '{}'", escape_filter_string(p)))
                    .collect();
                let filter = conditions.join(" OR ");
                let _ = table.delete(&filter).await;
            }

            table.add(Box::new(batches)).execute().await?;
        } else {
            let mut table_guard = self.table.write().await;
            if let Some(table) = &*table_guard {
                table.add(Box::new(batches)).execute().await?;
            } else {
                let table = self
                    .db
                    .create_table(TABLE_NAME, Box::new(batches))
                    .execute()
                    .await?;
                *table_guard = Some(table);
            }
        }

        Ok(())
    }


    pub async fn search_with_filter(
        &self,
        query_vector: &[f32],
        limit: usize,
        path_filter: Option<&str>,
    ) -> Result<Vec<SearchHit>> {
        let table_opt = (*self.table.read().await).clone();
        let Some(table) = table_opt else {
            return Ok(Vec::new());
        };

        // Fetch more results if filtering to compensate for filtered items
        let fetch_limit = if path_filter.is_some() {
            limit * 3
        } else {
            limit
        };

        let results = table
            .query()
            .nearest_to(query_vector)?
            .refine_factor(5)
            .limit(fetch_limit)
            .execute()
            .await?
            .try_collect::<Vec<RecordBatch>>()
            .await?;

        let mut hits = Vec::new();

        for batch in results {
            let ids = batch
                .column_by_name("id")
                .and_then(|c: &ArrayRef| c.as_any().downcast_ref::<StringArray>());
            let file_paths = batch
                .column_by_name("file_path")
                .and_then(|c: &ArrayRef| c.as_any().downcast_ref::<StringArray>());
            let contents = batch
                .column_by_name("content")
                .and_then(|c: &ArrayRef| c.as_any().downcast_ref::<StringArray>());
            let line_starts = batch
                .column_by_name("line_start")
                .and_then(|c: &ArrayRef| c.as_any().downcast_ref::<UInt32Array>());
            let line_ends = batch
                .column_by_name("line_end")
                .and_then(|c: &ArrayRef| c.as_any().downcast_ref::<UInt32Array>());
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c: &ArrayRef| c.as_any().downcast_ref::<Float32Array>());

            if let (
                Some(ids),
                Some(file_paths),
                Some(contents),
                Some(line_starts),
                Some(line_ends),
            ) = (ids, file_paths, contents, line_starts, line_ends)
            {
                for i in 0..batch.num_rows() {
                    let file_path = file_paths.value(i).to_string();

                    // Apply path filter if specified
                    if let Some(prefix) = path_filter {
                        if !file_path.starts_with(prefix) {
                            continue;
                        }
                    }

                    let distance = distances.map(|d: &Float32Array| d.value(i)).unwrap_or(0.0);
                    let score = 1.0 / (1.0 + distance);

                    hits.push(SearchHit {
                        id: ids.value(i).to_string(),
                        file_path,
                        content: contents.value(i).to_string(),
                        line_start: line_starts.value(i),
                        line_end: line_ends.value(i),
                        score,
                    });

                    // Stop if we have enough results
                    if hits.len() >= limit {
                        return Ok(hits);
                    }
                }
            }
        }

        Ok(hits)
    }

    /// Get total count of chunks (for health checks)
    pub async fn count(&self) -> Result<usize> {
        let table_opt = (*self.table.read().await).clone();
        if let Some(table) = table_opt {
            let count = table.count_rows(None).await?;
            Ok(count)
        } else {
            Ok(0)
        }
    }

    /// Health check - verifies database is accessible and table can be queried
    pub async fn health_check(&self) -> Result<()> {
        // Try to count rows - this verifies the table is accessible
        let _ = self.count().await?;
        Ok(())
    }

    /// Delete chunks for a file
    pub async fn delete_file(&self, file_path: &str) -> Result<()> {
        let table_opt = (*self.table.read().await).clone();
        if let Some(table) = table_opt {
            table
                .delete(&format!(
                    "file_path = '{}'",
                    escape_filter_string(file_path)
                ))
                .await?;
        }
        Ok(())
    }

    /// Drop the entire code_chunks table and recreate an empty one.
    /// Used by force reindex so orphan embeddings cannot survive a rebuild.
    pub async fn clear_all(&self) -> Result<()> {
        {
            let mut table_guard = self.table.write().await;
            *table_guard = None;
        }
        let names = self.db.table_names().execute().await?;
        if names.iter().any(|n| n == TABLE_NAME) {
            self.db.drop_table(TABLE_NAME, &[]).await?;
            tracing::info!("LanceDB: dropped table '{}'", TABLE_NAME);
        }
        self.ensure_table().await?;
        Ok(())
    }

    /// Create an IVF-PQ vector index for faster ANN search.
    /// Uses incremental logic: only rebuilds if row count grew >= 20% since last build.
    pub async fn create_vector_index_incremental(
        &self,
        sqlite: Option<&crate::storage::SqliteStorage>,
    ) -> Result<()> {
        let table_opt = (*self.table.read().await).clone();
        let Some(table) = table_opt else {
            return Ok(());
        };

        let row_count = table.count_rows(None).await?;
        if row_count < 256 {
            tracing::debug!(
                "Skipping IVF index creation: only {} rows (need >= 256)",
                row_count
            );
            return Ok(());
        }

        // Check if rebuild is needed (>= 20% growth since last build)
        if let Some(sq) = sqlite {
            if let Ok(Some(last_str)) = sq.get_index_meta("lance_last_index_rows").await {
                if let Ok(last_count) = last_str.parse::<usize>() {
                    if last_count > 0 {
                        let growth = (row_count as f64 - last_count as f64) / last_count as f64;
                        if growth < 0.20 {
                            tracing::debug!(
                                "Skipping IVF rebuild: {} rows, last built at {} ({:.1}% growth < 20%)",
                                row_count, last_count, growth * 100.0
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }

        // Optimized partition count: sqrt(rows) clamped to reasonable range.
        // More partitions = faster search, slower build. Cap at 256 for large repos.
        let num_partitions = ((row_count as f64).sqrt() as u32).clamp(4, 256);
        // Sub-quantizers: 16 for BGE-small (384 dims, 384/16 = 24 dims per sub-quantizer)
        let num_sub_vectors = 16u32;

        tracing::info!(
            "Creating IVF-PQ index on {} rows ({} partitions, {} sub-vectors)...",
            row_count,
            num_partitions,
            num_sub_vectors,
        );

        let index = lancedb::index::vector::IvfPqIndexBuilder::default()
            .distance_type(DistanceType::Cosine)
            .num_partitions(num_partitions)
            .num_sub_vectors(num_sub_vectors);

        table
            .create_index(&["vector"], Index::IvfPq(index))
            .execute()
            .await?;

        // Save row count for next incremental check
        if let Some(sq) = sqlite {
            let _ = sq
                .set_index_meta("lance_last_index_rows", &row_count.to_string())
                .await;
        }

        tracing::info!("IVF-PQ index created");
        Ok(())
    }

    /// Compact small fragments and prune old versions to reduce read amplification.
    pub async fn compact(&self) -> Result<()> {
        let table_opt = (*self.table.read().await).clone();
        let Some(table) = table_opt else {
            return Ok(());
        };

        let stats = table.optimize(OptimizeAction::All).await?;

        tracing::info!(
            "LanceDB compaction: compacted {} fragments, pruned {} bytes",
            stats
                .compaction
                .as_ref()
                .map(|c| c.fragments_removed)
                .unwrap_or(0),
            stats.prune.as_ref().map(|p| p.bytes_removed).unwrap_or(0),
        );

        Ok(())
    }
}

/// Search result from LanceDB
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchHit {
    pub id: String,
    pub file_path: String,
    pub content: String,
    pub line_start: u32,
    pub line_end: u32,
    pub score: f32,
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TEST_VECTOR_DIM: usize = 8; // Small dimension for fast tests

    async fn create_test_storage() -> (LanceStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("lance_test");
        let storage = LanceStorage::new(db_path.to_str().unwrap(), TEST_VECTOR_DIM)
            .await
            .unwrap();
        (storage, temp_dir)
    }

    fn make_chunk(
        id: &str,
        file_path: &str,
        content: &str,
        line_start: u32,
        line_end: u32,
    ) -> CodeChunk {
        CodeChunk {
            id: id.to_string(),
            file_path: file_path.to_string(),
            content: content.to_string(),
            line_start,
            line_end,
            symbol_name: Some("test_symbol".to_string()),
            symbol_kind: Some(crate::models::chunk::SymbolKind::Function),
            symbol_path: None,
            scopes: Vec::new(),
        }
    }

    fn random_vector(dim: usize) -> Vec<f32> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::time::Instant::now().hash(&mut hasher);
        let seed = hasher.finish();

        (0..dim)
            .map(|i| ((seed.wrapping_add(i as u64) % 1000) as f32 / 1000.0) - 0.5)
            .collect()
    }

    fn normalize(v: &mut [f32]) {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
    }

    // -------------------------------------------------------------------------
    // Basic storage tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_storage_creation() {
        let (storage, _temp) = create_test_storage().await;
        assert!(storage.table.read().await.is_none()); // No table until first insert
    }

    #[tokio::test]
    async fn test_count_empty() {
        let (storage, _temp) = create_test_storage().await;
        let count = storage.count().await.unwrap();
        assert_eq!(count, 0);
    }

    // -------------------------------------------------------------------------
    // Upsert tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_upsert_single_chunk() {
        let (storage, _temp) = create_test_storage().await;

        let chunk = make_chunk("chunk1", "/test.rs", "fn test() {}", 1, 3);
        let embedding = random_vector(TEST_VECTOR_DIM);

        storage.upsert_chunks(&[chunk], &[embedding]).await.unwrap();

        let count = storage.count().await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_upsert_multiple_chunks() {
        let (storage, _temp) = create_test_storage().await;

        let chunks = vec![
            make_chunk("c1", "/file1.rs", "code1", 1, 5),
            make_chunk("c2", "/file1.rs", "code2", 6, 10),
            make_chunk("c3", "/file2.rs", "code3", 1, 3),
        ];
        let embeddings: Vec<_> = (0..3).map(|_| random_vector(TEST_VECTOR_DIM)).collect();

        storage.upsert_chunks(&chunks, &embeddings).await.unwrap();

        let count = storage.count().await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_upsert_replaces_file_chunks() {
        let (storage, _temp) = create_test_storage().await;

        // Insert initial chunks
        let chunks = vec![
            make_chunk("old1", "/file.rs", "old code 1", 1, 5),
            make_chunk("old2", "/file.rs", "old code 2", 6, 10),
        ];
        let embeddings: Vec<_> = (0..2).map(|_| random_vector(TEST_VECTOR_DIM)).collect();
        storage.upsert_chunks(&chunks, &embeddings).await.unwrap();

        assert_eq!(storage.count().await.unwrap(), 2);

        // Upsert with new chunks for same file - should replace
        let new_chunks = vec![make_chunk("new1", "/file.rs", "new code", 1, 3)];
        let new_embeddings = vec![random_vector(TEST_VECTOR_DIM)];
        storage
            .upsert_chunks(&new_chunks, &new_embeddings)
            .await
            .unwrap();

        // Should have only 1 chunk now (old ones replaced)
        assert_eq!(storage.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_upsert_empty() {
        let (storage, _temp) = create_test_storage().await;

        // Empty upsert should succeed without error
        storage.upsert_chunks(&[], &[]).await.unwrap();
        assert_eq!(storage.count().await.unwrap(), 0);
    }

    // -------------------------------------------------------------------------
    // Search tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_search_empty_table() {
        let (storage, _temp) = create_test_storage().await;

        let query = random_vector(TEST_VECTOR_DIM);
        let results = storage.search_with_filter(&query, 10, None).await.unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_returns_results() {
        let (storage, _temp) = create_test_storage().await;

        let chunks = vec![
            make_chunk("c1", "/file.rs", "function one", 1, 5),
            make_chunk("c2", "/file.rs", "function two", 6, 10),
        ];
        let embeddings: Vec<_> = (0..2).map(|_| random_vector(TEST_VECTOR_DIM)).collect();
        storage.upsert_chunks(&chunks, &embeddings).await.unwrap();

        let query = random_vector(TEST_VECTOR_DIM);
        let results = storage.search_with_filter(&query, 10, None).await.unwrap();

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_respects_limit() {
        let (storage, _temp) = create_test_storage().await;

        // Insert 10 chunks
        let chunks: Vec<_> = (0..10)
            .map(|i| {
                make_chunk(
                    &format!("c{}", i),
                    "/file.rs",
                    &format!("code {}", i),
                    i,
                    i + 1,
                )
            })
            .collect();
        let embeddings: Vec<_> = (0..10).map(|_| random_vector(TEST_VECTOR_DIM)).collect();
        storage.upsert_chunks(&chunks, &embeddings).await.unwrap();

        let query = random_vector(TEST_VECTOR_DIM);
        let results = storage.search_with_filter(&query, 3, None).await.unwrap();

        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_search_finds_similar() {
        let (storage, _temp) = create_test_storage().await;

        // Create a specific embedding and search for it
        let mut target_embedding = vec![1.0f32; TEST_VECTOR_DIM];
        normalize(&mut target_embedding);

        let mut different_embedding = vec![-1.0f32; TEST_VECTOR_DIM];
        normalize(&mut different_embedding);

        let chunks = vec![
            make_chunk("similar", "/file.rs", "similar code", 1, 5),
            make_chunk("different", "/file.rs", "different code", 6, 10),
        ];
        let embeddings = vec![target_embedding.clone(), different_embedding];
        storage.upsert_chunks(&chunks, &embeddings).await.unwrap();

        // Search with the target embedding - should find "similar" first
        let results = storage.search_with_filter(&target_embedding, 2, None).await.unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "similar");
        assert!(results[0].score > results[1].score);
    }

    #[tokio::test]
    async fn test_search_hit_fields() {
        let (storage, _temp) = create_test_storage().await;

        let chunk = make_chunk("test_id", "/path/to/file.rs", "fn main() {}", 10, 20);
        let embedding = random_vector(TEST_VECTOR_DIM);
        storage
            .upsert_chunks(&[chunk], std::slice::from_ref(&embedding))
            .await
            .unwrap();

        let results = storage.search_with_filter(&embedding, 1, None).await.unwrap();

        assert_eq!(results.len(), 1);
        let hit = &results[0];
        assert_eq!(hit.id, "test_id");
        assert_eq!(hit.file_path, "/path/to/file.rs");
        assert_eq!(hit.content, "fn main() {}");
        assert_eq!(hit.line_start, 10);
        assert_eq!(hit.line_end, 20);
        assert!(hit.score > 0.0);
    }

    // -------------------------------------------------------------------------
    // Delete tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_file() {
        let (storage, _temp) = create_test_storage().await;

        let chunks = vec![
            make_chunk("c1", "/keep.rs", "code1", 1, 5),
            make_chunk("c2", "/delete.rs", "code2", 1, 5),
            make_chunk("c3", "/delete.rs", "code3", 6, 10),
        ];
        let embeddings: Vec<_> = (0..3).map(|_| random_vector(TEST_VECTOR_DIM)).collect();
        storage.upsert_chunks(&chunks, &embeddings).await.unwrap();

        assert_eq!(storage.count().await.unwrap(), 3);

        storage.delete_file("/delete.rs").await.unwrap();

        // Should have only 1 chunk left
        assert_eq!(storage.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_file() {
        let (storage, _temp) = create_test_storage().await;

        // Delete on empty table should not error
        let result = storage.delete_file("/nonexistent.rs").await;
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_special_characters_in_path() {
        let (storage, _temp) = create_test_storage().await;

        // Path with special characters that need escaping
        let chunk = make_chunk("c1", "/path/with'quote/file.rs", "code", 1, 5);
        let embedding = random_vector(TEST_VECTOR_DIM);

        storage.upsert_chunks(&[chunk], &[embedding]).await.unwrap();
        assert_eq!(storage.count().await.unwrap(), 1);

        // Delete should handle escaped quotes
        storage
            .delete_file("/path/with'quote/file.rs")
            .await
            .unwrap();
        assert_eq!(storage.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_unicode_content() {
        let (storage, _temp) = create_test_storage().await;

        let chunk = make_chunk("unicode", "/test.rs", "// Привет мир! 你好世界", 1, 5);
        let embedding = random_vector(TEST_VECTOR_DIM);

        storage
            .upsert_chunks(&[chunk], std::slice::from_ref(&embedding))
            .await
            .unwrap();

        let results = storage.search_with_filter(&embedding, 1, None).await.unwrap();
        assert_eq!(results[0].content, "// Привет мир! 你好世界");
    }

    #[tokio::test]
    async fn test_escape_filter_string() {
        assert_eq!(escape_filter_string("normal"), "normal");
        assert_eq!(escape_filter_string("with'quote"), "with''quote");
        assert_eq!(escape_filter_string("with\0null"), "withnull");
        assert_eq!(escape_filter_string("a'b\0c'd"), "a''bc''d");
    }

    // -------------------------------------------------------------------------
    // Schema tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_schema_dimensions() {
        let temp_dir = TempDir::new().unwrap();

        // Test with different dimensions
        let db_path = temp_dir.path().join("lance_384");
        let storage = LanceStorage::new(db_path.to_str().unwrap(), 384)
            .await
            .unwrap();
        let schema = storage.schema();

        let vector_field = schema.field_with_name("vector").unwrap();
        if let DataType::FixedSizeList(_, size) = vector_field.data_type() {
            assert_eq!(*size, 384);
        } else {
            panic!("Expected FixedSizeList for vector field");
        }
    }
}
