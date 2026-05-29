# Feature: get_index_status - Видимость состояния индекса

**ID:** PHASE0-001  
**Priority:** 🔥🔥🔥 Critical  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)

---

## 📋 Описание

Инструмент для получения полной информации о текущем состоянии индекса gofer MCP. Позволяет понять:
- Что проиндексировано, что нет
- Степень завершенности индексации
- Когда была последняя синхронизация
- Статус очереди индексации

### Проблема

Сейчас нет способа узнать:
- Завершилась ли индексация проекта
- Какие файлы еще не проиндексированы
- Почему поиск не находит какой-то символ (может он не проиндексирован?)
- Сколько времени назад был последний sync

Это создает неопределенность и приводит к ситуациям типа "почему gofer не видит мой код?"

### Решение

MCP tool `get_index_status` который возвращает детальную информацию о состоянии индекса.

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Дать visibility в состояние индекса
- ✅ Помочь диагностировать проблемы с поиском
- ✅ Показать прогресс индексации
- ✅ Предупредить если индекс устарел

### Non-Goals
- ❌ Не исправляет проблемы индексации (это делает `force_reindex`)
- ❌ Не запускает индексацию автоматически
- ❌ Не показывает historical data (только current state)

---

## 🏗️ Архитектура

### Компоненты

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│     get_index_status()                  │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Index Status   │
        │    Collector    │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│ SQLite   │ │Lance │ │  File     │
│  Stats   │ │ DB   │ │  Watcher  │
└──────────┘ └──────┘ └───────────┘
```

### Data Flow

```
1. MCP Request: get_index_status()
   ↓
2. Query SQLite для metadata:
   - SELECT COUNT(*) FROM files
   - SELECT COUNT(*) FROM symbols
   - SELECT COUNT(*) FROM chunks
   - SELECT last_sync FROM index_metadata
   ↓
3. Query LanceDB для vectors:
   - Count embeddings
   ↓
4. Check File Watcher queue:
   - Pending files
   ↓
5. Calculate completeness %
   ↓
6. Return JSON response
```

---

## 📊 Data Model

### Новая таблица: index_metadata

```sql
-- migrations/014_index_metadata.sql
CREATE TABLE index_metadata (
    id INTEGER PRIMARY KEY CHECK (id = 1),  -- Singleton row
    last_full_sync DATETIME,
    last_incremental_sync DATETIME,
    total_files_scanned INTEGER NOT NULL DEFAULT 0,
    total_files_indexed INTEGER NOT NULL DEFAULT 0,
    total_symbols_extracted INTEGER NOT NULL DEFAULT 0,
    total_chunks_created INTEGER NOT NULL DEFAULT 0,
    total_embeddings_generated INTEGER NOT NULL DEFAULT 0,
    indexer_version TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Insert initial row
INSERT INTO index_metadata (
    id, 
    indexer_version
) VALUES (
    1, 
    '0.1.0'
);

-- Index for quick queries
CREATE INDEX idx_metadata_sync ON index_metadata(last_incremental_sync);
```

### Расширение существующих таблиц

```sql
-- Add indexed_at timestamp to files table
ALTER TABLE files ADD COLUMN indexed_at DATETIME;
ALTER TABLE files ADD COLUMN index_status TEXT DEFAULT 'pending';
  -- Values: 'pending', 'indexing', 'completed', 'failed'

CREATE INDEX idx_files_status ON files(index_status);
CREATE INDEX idx_files_indexed_at ON files(indexed_at);
```

---

## 🔧 API Specification

### MCP Tool Definition

```json
{
  "name": "get_index_status",
  "description": "Получить текущее состояние индекса gofer MCP: что проиндексировано, статус очереди, completeness metrics",
  "inputSchema": {
    "type": "object",
    "properties": {
      "detailed": {
        "type": "boolean",
        "default": false,
        "description": "Включить детальную информацию по модулям"
      },
      "include_queue": {
        "type": "boolean",
        "default": true,
        "description": "Включить информацию об очереди индексации"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct IndexStatus {
    // Overall status
    pub status: IndexHealth,  // Healthy, Degraded, Unhealthy
    pub last_sync: Option<DateTime<Utc>>,
    pub age_minutes: i64,
    
    // Completeness metrics
    pub completeness: CompletenessMetrics,
    
    // Component stats
    pub files: FileIndexStats,
    pub symbols: SymbolIndexStats,
    pub embeddings: EmbeddingIndexStats,
    pub summaries: SummaryIndexStats,
    
    // Queue information
    pub queue: Option<QueueStatus>,
    
    // Detailed breakdown (if requested)
    pub modules: Option<Vec<ModuleIndexStatus>>,
    
    // Warnings and recommendations
    pub warnings: Vec<IndexWarning>,
    pub recommendations: Vec<String>,
}

#[derive(Serialize)]
pub enum IndexHealth {
    Healthy,    // Last sync < 1 hour, queue empty, > 95% complete
    Degraded,   // Last sync < 24 hours, small queue, > 80% complete
    Unhealthy,  // Last sync > 24 hours, large queue, < 80% complete
}

#[derive(Serialize)]
pub struct CompletenessMetrics {
    pub overall_percent: f32,  // 0.0 - 100.0
    pub files_percent: f32,
    pub symbols_percent: f32,
    pub embeddings_percent: f32,
    pub summaries_percent: f32,
}

#[derive(Serialize)]
pub struct FileIndexStats {
    pub total_in_project: usize,      // From file system scan
    pub indexed: usize,                // In database
    pub pending: usize,                // Not yet indexed
    pub failed: usize,                 // Failed to index
    pub oldest_indexed: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct SymbolIndexStats {
    pub total: usize,
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub other: usize,
}

#[derive(Serialize)]
pub struct EmbeddingIndexStats {
    pub total_chunks: usize,
    pub total_vectors: usize,
    pub vector_dimension: usize,
    pub avg_chunk_size: f32,
}

#[derive(Serialize)]
pub struct SummaryIndexStats {
    pub files_with_summaries: usize,
    pub files_without_summaries: usize,
    pub percent_covered: f32,
}

#[derive(Serialize)]
pub struct QueueStatus {
    pub pending_files: usize,
    pub files_in_queue: Vec<QueuedFile>,
    pub estimated_time_seconds: u64,
    pub is_processing: bool,
}

#[derive(Serialize)]
pub struct QueuedFile {
    pub path: String,
    pub size_bytes: u64,
    pub priority: Priority,
    pub added_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub enum Priority {
    High,
    Normal,
    Low,
}

#[derive(Serialize)]
pub struct ModuleIndexStatus {
    pub module_name: String,
    pub files_indexed: usize,
    pub files_total: usize,
    pub completeness_percent: f32,
}

#[derive(Serialize)]
pub struct IndexWarning {
    pub severity: WarningSeverity,
    pub message: String,
    pub recommendation: String,
}

#[derive(Serialize)]
pub enum WarningSeverity {
    Info,
    Warning,
    Error,
}
```

### Example Response

```json
{
  "status": "Healthy",
  "last_sync": "2026-02-16T10:30:00Z",
  "age_minutes": 15,
  "completeness": {
    "overall_percent": 96.5,
    "files_percent": 100.0,
    "symbols_percent": 98.2,
    "embeddings_percent": 95.0,
    "summaries_percent": 92.3
  },
  "files": {
    "total_in_project": 44,
    "indexed": 44,
    "pending": 0,
    "failed": 0,
    "oldest_indexed": "2026-02-15T18:20:00Z"
  },
  "symbols": {
    "total": 1250,
    "functions": 850,
    "structs": 200,
    "enums": 80,
    "traits": 60,
    "other": 60
  },
  "embeddings": {
    "total_chunks": 597,
    "total_vectors": 597,
    "vector_dimension": 384,
    "avg_chunk_size": 512.3
  },
  "summaries": {
    "files_with_summaries": 40,
    "files_without_summaries": 4,
    "percent_covered": 90.9
  },
  "queue": {
    "pending_files": 0,
    "files_in_queue": [],
    "estimated_time_seconds": 0,
    "is_processing": false
  },
  "warnings": [],
  "recommendations": [
    "Index is healthy and up-to-date",
    "4 files missing summaries - consider running summarization"
  ]
}
```

---

## 💻 Implementation Details

### Step 1: Database Schema Migration

```rust
// src/storage/migrations/014_index_metadata.sql
// (см. выше Data Model)

// Apply migration
impl SqliteStorage {
    pub async fn migrate_index_metadata(&self) -> Result<()> {
        self.conn.execute_batch(
            include_str!("migrations/014_index_metadata.sql")
        )?;
        Ok(())
    }
}
```

### Step 2: Index Metadata Tracker

```rust
// src/indexer/metadata.rs

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

pub struct IndexMetadataTracker {
    pool: SqlitePool,
}

impl IndexMetadataTracker {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    
    /// Update metadata after file indexing
    pub async fn record_file_indexed(&self, file_id: i64) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE files 
            SET 
                indexed_at = CURRENT_TIMESTAMP,
                index_status = 'completed'
            WHERE id = ?
            "#,
            file_id
        )
        .execute(&self.pool)
        .await?;
        
        // Update global metadata
        self.update_global_metadata().await?;
        
        Ok(())
    }
    
    /// Update global metadata counters
    async fn update_global_metadata(&self) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE index_metadata
            SET
                last_incremental_sync = CURRENT_TIMESTAMP,
                total_files_indexed = (SELECT COUNT(*) FROM files WHERE index_status = 'completed'),
                total_symbols_extracted = (SELECT COUNT(*) FROM symbols),
                total_chunks_created = (SELECT COUNT(*) FROM chunks),
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    /// Mark file as failed
    pub async fn record_file_failed(&self, file_id: i64, error: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE files 
            SET 
                index_status = 'failed',
                error_message = ?
            WHERE id = ?
            "#,
            error,
            file_id
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
}
```

### Step 3: Status Collector

```rust
// src/indexer/status_collector.rs

use crate::storage::SqliteStorage;
use crate::storage::lance::LanceStorage;
use crate::watcher::FileWatcher;

pub struct IndexStatusCollector {
    sqlite: SqliteStorage,
    lance: LanceStorage,
    watcher: FileWatcher,
}

impl IndexStatusCollector {
    pub fn new(
        sqlite: SqliteStorage, 
        lance: LanceStorage,
        watcher: FileWatcher
    ) -> Self {
        Self { sqlite, lance, watcher }
    }
    
    pub async fn collect_status(&self, detailed: bool) -> Result<IndexStatus> {
        // 1. Get last sync time
        let metadata = self.get_metadata().await?;
        
        // 2. Calculate age
        let age_minutes = metadata.last_sync
            .map(|t| (Utc::now() - t).num_minutes())
            .unwrap_or(i64::MAX);
        
        // 3. Get file stats
        let files = self.collect_file_stats().await?;
        
        // 4. Get symbol stats
        let symbols = self.collect_symbol_stats().await?;
        
        // 5. Get embedding stats
        let embeddings = self.collect_embedding_stats().await?;
        
        // 6. Get summary stats
        let summaries = self.collect_summary_stats().await?;
        
        // 7. Calculate completeness
        let completeness = self.calculate_completeness(
            &files, &symbols, &embeddings, &summaries
        );
        
        // 8. Get queue status
        let queue = self.collect_queue_status().await?;
        
        // 9. Determine overall health
        let status = self.determine_health(
            age_minutes, 
            &completeness, 
            &queue
        );
        
        // 10. Generate warnings and recommendations
        let (warnings, recommendations) = self.generate_warnings_and_recommendations(
            &status,
            age_minutes,
            &completeness,
            &files,
            &summaries
        );
        
        // 11. Detailed module breakdown (if requested)
        let modules = if detailed {
            Some(self.collect_module_stats().await?)
        } else {
            None
        };
        
        Ok(IndexStatus {
            status,
            last_sync: metadata.last_sync,
            age_minutes,
            completeness,
            files,
            symbols,
            embeddings,
            summaries,
            queue: Some(queue),
            modules,
            warnings,
            recommendations,
        })
    }
    
    async fn get_metadata(&self) -> Result<IndexMetadata> {
        let row = sqlx::query!(
            r#"
            SELECT 
                last_incremental_sync,
                total_files_indexed,
                total_symbols_extracted,
                total_chunks_created
            FROM index_metadata
            WHERE id = 1
            "#
        )
        .fetch_one(&self.sqlite.pool)
        .await?;
        
        Ok(IndexMetadata {
            last_sync: row.last_incremental_sync,
            files_indexed: row.total_files_indexed as usize,
            symbols_extracted: row.total_symbols_extracted as usize,
            chunks_created: row.total_chunks_created as usize,
        })
    }
    
    async fn collect_file_stats(&self) -> Result<FileIndexStats> {
        // Count files in database
        let indexed = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM files WHERE index_status = 'completed'"
        )
        .fetch_one(&self.sqlite.pool)
        .await? as usize;
        
        let pending = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM files WHERE index_status = 'pending'"
        )
        .fetch_one(&self.sqlite.pool)
        .await? as usize;
        
        let failed = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM files WHERE index_status = 'failed'"
        )
        .fetch_one(&self.sqlite.pool)
        .await? as usize;
        
        // Count files in project (file system scan)
        let total_in_project = self.scan_project_files().await?;
        
        // Get oldest indexed file
        let oldest_indexed = sqlx::query_scalar!(
            "SELECT MIN(indexed_at) FROM files WHERE index_status = 'completed'"
        )
        .fetch_optional(&self.sqlite.pool)
        .await?
        .flatten();
        
        Ok(FileIndexStats {
            total_in_project,
            indexed,
            pending,
            failed,
            oldest_indexed,
        })
    }
    
    async fn collect_symbol_stats(&self) -> Result<SymbolIndexStats> {
        let rows = sqlx::query!(
            r#"
            SELECT 
                kind,
                COUNT(*) as count
            FROM symbols
            GROUP BY kind
            "#
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        let mut stats = SymbolIndexStats {
            total: 0,
            functions: 0,
            structs: 0,
            enums: 0,
            traits: 0,
            other: 0,
        };
        
        for row in rows {
            let count = row.count as usize;
            stats.total += count;
            
            match row.kind.as_deref() {
                Some("function") => stats.functions = count,
                Some("struct") => stats.structs = count,
                Some("enum") => stats.enums = count,
                Some("trait") => stats.traits = count,
                _ => stats.other += count,
            }
        }
        
        Ok(stats)
    }
    
    async fn collect_embedding_stats(&self) -> Result<EmbeddingIndexStats> {
        // Query LanceDB
        let total_vectors = self.lance.count_vectors().await?;
        let vector_dimension = self.lance.get_vector_dimension().await?;
        
        // Query chunks from SQLite
        let chunks_data = sqlx::query!(
            r#"
            SELECT 
                COUNT(*) as total,
                AVG(LENGTH(content)) as avg_size
            FROM chunks
            "#
        )
        .fetch_one(&self.sqlite.pool)
        .await?;
        
        Ok(EmbeddingIndexStats {
            total_chunks: chunks_data.total as usize,
            total_vectors,
            vector_dimension,
            avg_chunk_size: chunks_data.avg_size.unwrap_or(0.0) as f32,
        })
    }
    
    async fn collect_summary_stats(&self) -> Result<SummaryIndexStats> {
        let with_summaries = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM files WHERE summary IS NOT NULL"
        )
        .fetch_one(&self.sqlite.pool)
        .await? as usize;
        
        let without_summaries = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM files WHERE summary IS NULL"
        )
        .fetch_one(&self.sqlite.pool)
        .await? as usize;
        
        let total = with_summaries + without_summaries;
        let percent_covered = if total > 0 {
            (with_summaries as f32 / total as f32) * 100.0
        } else {
            0.0
        };
        
        Ok(SummaryIndexStats {
            files_with_summaries: with_summaries,
            files_without_summaries: without_summaries,
            percent_covered,
        })
    }
    
    async fn collect_queue_status(&self) -> Result<QueueStatus> {
        let queue = self.watcher.get_queue_status().await?;
        
        Ok(QueueStatus {
            pending_files: queue.len(),
            files_in_queue: queue,
            estimated_time_seconds: self.estimate_queue_time(&queue),
            is_processing: self.watcher.is_processing().await,
        })
    }
    
    fn calculate_completeness(
        &self,
        files: &FileIndexStats,
        symbols: &SymbolIndexStats,
        embeddings: &EmbeddingIndexStats,
        summaries: &SummaryIndexStats,
    ) -> CompletenessMetrics {
        let files_percent = if files.total_in_project > 0 {
            (files.indexed as f32 / files.total_in_project as f32) * 100.0
        } else {
            100.0
        };
        
        // Symbols: heuristic based on typical file → symbols ratio
        let expected_symbols = files.indexed * 10; // ~10 symbols per file
        let symbols_percent = if expected_symbols > 0 {
            ((symbols.total as f32 / expected_symbols as f32) * 100.0).min(100.0)
        } else {
            0.0
        };
        
        // Embeddings: should match chunks
        let embeddings_percent = if embeddings.total_chunks > 0 {
            ((embeddings.total_vectors as f32 / embeddings.total_chunks as f32) * 100.0).min(100.0)
        } else {
            0.0
        };
        
        let summaries_percent = summaries.percent_covered;
        
        // Overall: weighted average
        let overall_percent = 
            files_percent * 0.4 +
            symbols_percent * 0.3 +
            embeddings_percent * 0.2 +
            summaries_percent * 0.1;
        
        CompletenessMetrics {
            overall_percent,
            files_percent,
            symbols_percent,
            embeddings_percent,
            summaries_percent,
        }
    }
    
    fn determine_health(
        &self,
        age_minutes: i64,
        completeness: &CompletenessMetrics,
        queue: &QueueStatus,
    ) -> IndexHealth {
        if age_minutes < 60 
            && completeness.overall_percent > 95.0 
            && queue.pending_files == 0 
        {
            IndexHealth::Healthy
        } else if age_minutes < 1440  // 24 hours
            && completeness.overall_percent > 80.0
            && queue.pending_files < 10
        {
            IndexHealth::Degraded
        } else {
            IndexHealth::Unhealthy
        }
    }
    
    fn generate_warnings_and_recommendations(
        &self,
        health: &IndexHealth,
        age_minutes: i64,
        completeness: &CompletenessMetrics,
        files: &FileIndexStats,
        summaries: &SummaryIndexStats,
    ) -> (Vec<IndexWarning>, Vec<String>) {
        let mut warnings = Vec::new();
        let mut recommendations = Vec::new();
        
        // Check age
        if age_minutes > 60 {
            warnings.push(IndexWarning {
                severity: WarningSeverity::Warning,
                message: format!("Index is {} minutes old", age_minutes),
                recommendation: "Consider running force_reindex to update".into(),
            });
        }
        
        // Check failed files
        if files.failed > 0 {
            warnings.push(IndexWarning {
                severity: WarningSeverity::Error,
                message: format!("{} files failed to index", files.failed),
                recommendation: "Check logs for errors and fix issues".into(),
            });
        }
        
        // Check completeness
        if completeness.overall_percent < 95.0 {
            warnings.push(IndexWarning {
                severity: WarningSeverity::Warning,
                message: format!("Index is only {:.1}% complete", completeness.overall_percent),
                recommendation: "Wait for indexing to complete or run force_reindex".into(),
            });
        }
        
        // Summaries
        if summaries.files_without_summaries > 0 {
            recommendations.push(
                format!(
                    "{} files missing summaries - consider running summarization",
                    summaries.files_without_summaries
                )
            );
        }
        
        // Overall health
        match health {
            IndexHealth::Healthy => {
                recommendations.push("Index is healthy and up-to-date".into());
            }
            IndexHealth::Degraded => {
                recommendations.push("Index is slightly outdated, consider refreshing".into());
            }
            IndexHealth::Unhealthy => {
                recommendations.push("Index needs attention - run force_reindex".into());
            }
        }
        
        (warnings, recommendations)
    }
    
    async fn scan_project_files(&self) -> Result<usize> {
        // Use existing watcher to scan project
        let files = self.watcher.scan_workspace().await?;
        Ok(files.len())
    }
    
    fn estimate_queue_time(&self, queue: &[QueuedFile]) -> u64 {
        // Heuristic: ~1 second per file on average
        queue.len() as u64
    }
    
    async fn collect_module_stats(&self) -> Result<Vec<ModuleIndexStatus>> {
        // Group files by module (top-level directory)
        let rows = sqlx::query!(
            r#"
            SELECT 
                SUBSTR(path, 1, INSTR(path || '/', '/') - 1) as module,
                COUNT(*) as total,
                SUM(CASE WHEN index_status = 'completed' THEN 1 ELSE 0 END) as indexed
            FROM files
            GROUP BY module
            "#
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        let modules = rows.into_iter().map(|row| {
            let total = row.total as usize;
            let indexed = row.indexed.unwrap_or(0) as usize;
            let completeness_percent = if total > 0 {
                (indexed as f32 / total as f32) * 100.0
            } else {
                0.0
            };
            
            ModuleIndexStatus {
                module_name: row.module.unwrap_or_else(|| "root".to_string()),
                files_indexed: indexed,
                files_total: total,
                completeness_percent,
            }
        }).collect();
        
        Ok(modules)
    }
}
```

### Step 4: MCP Tool Handler

```rust
// src/daemon/tools.rs

pub async fn handle_get_index_status(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
    lance: &LanceStorage,
    watcher: &FileWatcher,
) -> Result<Value> {
    let detailed = args.get("detailed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    let include_queue = args.get("include_queue")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let collector = IndexStatusCollector::new(
        sqlite.clone(),
        lance.clone(),
        watcher.clone(),
    );
    
    let mut status = collector.collect_status(detailed).await?;
    
    // Remove queue if not requested
    if !include_queue {
        status.queue = None;
    }
    
    Ok(serde_json::to_value(status)?)
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_collect_file_stats() {
        let collector = setup_test_collector().await;
        
        let stats = collector.collect_file_stats().await.unwrap();
        
        assert_eq!(stats.total_in_project, 44);
        assert_eq!(stats.indexed, 44);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.failed, 0);
    }
    
    #[tokio::test]
    async fn test_calculate_completeness() {
        let collector = setup_test_collector().await;
        
        let files = FileIndexStats {
            total_in_project: 100,
            indexed: 95,
            pending: 5,
            failed: 0,
            oldest_indexed: None,
        };
        
        let symbols = SymbolIndexStats {
            total: 950,
            functions: 700,
            structs: 150,
            enums: 50,
            traits: 30,
            other: 20,
        };
        
        let embeddings = EmbeddingIndexStats {
            total_chunks: 500,
            total_vectors: 500,
            vector_dimension: 384,
            avg_chunk_size: 512.0,
        };
        
        let summaries = SummaryIndexStats {
            files_with_summaries: 90,
            files_without_summaries: 5,
            percent_covered: 94.7,
        };
        
        let completeness = collector.calculate_completeness(
            &files, &symbols, &embeddings, &summaries
        );
        
        assert!(completeness.overall_percent > 90.0);
        assert_eq!(completeness.files_percent, 95.0);
    }
    
    #[tokio::test]
    async fn test_determine_health() {
        let collector = setup_test_collector().await;
        
        // Healthy
        let completeness = CompletenessMetrics {
            overall_percent: 96.0,
            files_percent: 100.0,
            symbols_percent: 95.0,
            embeddings_percent: 95.0,
            summaries_percent: 90.0,
        };
        let queue = QueueStatus {
            pending_files: 0,
            files_in_queue: vec![],
            estimated_time_seconds: 0,
            is_processing: false,
        };
        
        let health = collector.determine_health(30, &completeness, &queue);
        assert!(matches!(health, IndexHealth::Healthy));
        
        // Degraded
        let health = collector.determine_health(120, &completeness, &queue);
        assert!(matches!(health, IndexHealth::Degraded));
        
        // Unhealthy
        let completeness_low = CompletenessMetrics {
            overall_percent: 70.0,
            ..completeness
        };
        let health = collector.determine_health(1500, &completeness_low, &queue);
        assert!(matches!(health, IndexHealth::Unhealthy));
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_get_index_status_full_workflow() {
    let (sqlite, lance, watcher) = setup_test_environment().await;
    
    // 1. Empty index initially
    let status = IndexStatusCollector::new(sqlite.clone(), lance.clone(), watcher.clone())
        .collect_status(false)
        .await
        .unwrap();
    
    assert_eq!(status.files.indexed, 0);
    assert!(matches!(status.status, IndexHealth::Unhealthy));
    
    // 2. Index some files
    index_test_files(&sqlite, &lance).await;
    
    // 3. Check status again
    let status = IndexStatusCollector::new(sqlite.clone(), lance.clone(), watcher.clone())
        .collect_status(false)
        .await
        .unwrap();
    
    assert!(status.files.indexed > 0);
    assert!(status.completeness.overall_percent > 80.0);
    
    // 4. Check with detailed flag
    let status = IndexStatusCollector::new(sqlite, lance, watcher)
        .collect_status(true)
        .await
        .unwrap();
    
    assert!(status.modules.is_some());
    let modules = status.modules.unwrap();
    assert!(!modules.is_empty());
}
```

---

## 📈 Success Metrics

### Functional
- ✅ Возвращает accurate counts (files, symbols, embeddings)
- ✅ Correctly calculates completeness %
- ✅ Health status reflects actual index state
- ✅ Queue information is real-time

### Performance
- ⏱️ Response time < 500ms for basic query
- ⏱️ Response time < 2s for detailed query
- 💾 Minimal memory overhead

### Quality
- ✅ Warnings are actionable
- ✅ Recommendations are helpful
- ✅ No false positives in health status

---

## 📚 Usage Examples

### Basic Usage

```typescript
// Qoder AI assistant calling the tool
const status = await gofer.get_index_status();

if (status.status === 'Unhealthy') {
  console.log('Index needs attention!');
  console.log(`Last sync: ${status.age_minutes} minutes ago`);
  console.log(`Completeness: ${status.completeness.overall_percent}%`);
  
  // Show recommendations
  status.recommendations.forEach(rec => console.log(`💡 ${rec}`));
}
```

### Detailed Query

```typescript
const status = await gofer.get_index_status({ detailed: true });

// Show per-module breakdown
status.modules.forEach(mod => {
  console.log(`${mod.module_name}: ${mod.completeness_percent}% complete`);
  console.log(`  ${mod.files_indexed}/${mod.files_total} files`);
});
```

### Monitoring Dashboard

```typescript
// Poll index status periodically
setInterval(async () => {
  const status = await gofer.get_index_status({ include_queue: true });
  
  // Update metrics
  prometheus.gauge('gofer_index_completeness', status.completeness.overall_percent);
  prometheus.gauge('gofer_index_age_minutes', status.age_minutes);
  prometheus.gauge('gofer_queue_size', status.queue.pending_files);
  
  // Alert if unhealthy
  if (status.status === 'Unhealthy') {
    alert('gofer index is unhealthy!');
  }
}, 60000); // Every minute
```

---

## 🔄 Dependencies

### Required
- SQLite storage (existing)
- LanceDB vector storage (existing)
- File watcher (existing)

### Optional
- Prometheus (for metrics export)
- Alerting system (for health monitoring)

---

## 🚀 Rollout Plan

### Phase 1: Core Implementation (2 days)
- [ ] Database migration
- [ ] Index metadata tracker
- [ ] Status collector (basic)
- [ ] MCP tool handler

### Phase 2: Enhanced Features (0.5 day)
- [ ] Detailed module breakdown
- [ ] Queue status integration
- [ ] Warning generation

### Phase 3: Testing & Polish (0.5 day)
- [ ] Unit tests
- [ ] Integration tests
- [ ] Documentation
- [ ] User feedback

---

## 🐛 Known Issues / Limitations

1. **File system scan can be slow** for large projects (>10k files)
   - Mitigation: Cache scan results, update incrementally

2. **Module detection is basic** (top-level directory only)
   - Future: Support nested modules, language-specific module detection

3. **Completeness calculation is heuristic-based**
   - Future: More sophisticated estimation based on language and project type

---

## 📖 Related Documentation

- `002_validate_index.md` - Index validation tool
- `003_force_reindex.md` - Manual reindexing
- `../architecture/indexer.md` - Indexer architecture
- `../architecture/storage.md` - Storage layer

---

## ✅ Acceptance Criteria

- [ ] MCP tool `get_index_status` is callable
- [ ] Returns valid JSON response matching schema
- [ ] Completeness metrics are accurate (±2%)
- [ ] Health status reflects actual index state
- [ ] Response time < 500ms for basic query
- [ ] Response time < 2s for detailed query
- [ ] All unit tests pass
- [ ] Integration test passes
- [ ] Documentation is complete
- [ ] Code review approved

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD
