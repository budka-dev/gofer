# Feature: force_reindex - Принудительная переиндексация

**ID:** PHASE0-003  
**Priority:** 🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)  
**Depends On:** 001_get_index_status, 002_validate_index

---

## 📋 Описание

Инструмент для принудительной переиндексации файлов, модулей или всего проекта. Используется когда индекс устарел, поврежден или `validate_index` нашел проблемы.

### Проблема

Индекс может стать неактуальным или поврежденным:
- Файлы изменились, но file watcher пропустил события
- Индексация упала с ошибкой на половине проекта
- Обновлен embedder model (новая dimension)
- Изменился parser (новые языковые конструкции)
- Пользователь хочет перестроить индекс "с нуля"

Сейчас единственный способ - удалить БД и перезапустить daemon, что медленно и неудобно.

### Решение

MCP tool `force_reindex` с гибкими опциями:
- Переиндексировать конкретные файлы
- Переиндексировать модуль/директорию
- Переиндексировать весь проект
- Только embeddings (не трогая symbols)
- С приоритетом (high priority files first)

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Переиндексация файлов без перезапуска daemon
- ✅ Поддержка selective reindexing (конкретные файлы/модули)
- ✅ Priority queue (важные файлы первыми)
- ✅ Progress tracking (real-time progress updates)
- ✅ Graceful handling (не ломает текущие queries)

### Non-Goals
- ❌ Не делает automatic reindexing (это делает file watcher)
- ❌ Не валидирует код (это делает компилятор)
- ❌ Не оптимизирует индекс (дефрагментация etc)

---

## 🏗️ Архитектура

### Reindexing Pipeline

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│       force_reindex()                   │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Reindexing     │
        │    Manager      │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│Priority  │ │Index │ │ Progress  │
│  Queue   │ │Worker│ │  Tracker  │
└──────────┘ └──┬───┘ └───────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│ Parser   │ │Embed.│ │  Storage  │
│ (AST)    │ │Model │ │ (SQLite)  │
└──────────┘ └──────┘ └───────────┘
```

### State Management

```rust
// Reindexing task lifecycle

[Created] → [Queued] → [Running] → [Completed]
                ↓          ↓
              [Paused]  [Failed]
                ↓
              [Resumed]
```

---

## 📊 Data Model

### Reindexing Task Schema

```sql
-- migrations/015_reindexing_tasks.sql
CREATE TABLE reindexing_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL UNIQUE,
    scope TEXT NOT NULL,  -- 'file', 'directory', 'project', 'embeddings-only'
    target_paths TEXT NOT NULL,  -- JSON array of paths
    priority TEXT NOT NULL,  -- 'high', 'normal', 'low'
    status TEXT NOT NULL,  -- 'queued', 'running', 'paused', 'completed', 'failed'
    progress REAL NOT NULL DEFAULT 0.0,  -- 0.0 - 1.0
    items_total INTEGER NOT NULL DEFAULT 0,
    items_completed INTEGER NOT NULL DEFAULT 0,
    items_failed INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at DATETIME,
    completed_at DATETIME,
    estimated_duration_seconds INTEGER
);

CREATE INDEX idx_tasks_status ON reindexing_tasks(status);
CREATE INDEX idx_tasks_priority ON reindexing_tasks(priority);
CREATE INDEX idx_tasks_created ON reindexing_tasks(created_at);
```

### Task Items (Granular Tracking)

```sql
CREATE TABLE reindexing_task_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES reindexing_tasks(task_id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    status TEXT NOT NULL,  -- 'pending', 'processing', 'completed', 'failed'
    error_message TEXT,
    started_at DATETIME,
    completed_at DATETIME,
    duration_ms INTEGER
);

CREATE INDEX idx_task_items_task ON reindexing_task_items(task_id);
CREATE INDEX idx_task_items_status ON reindexing_task_items(status);
```

---

## 🔧 API Specification

### MCP Tool Definition

```json
{
  "name": "force_reindex",
  "description": "Force reindexing of files, directories, or entire project",
  "inputSchema": {
    "type": "object",
    "properties": {
      "scope": {
        "type": "string",
        "enum": ["file", "files", "directory", "project", "embeddings-only"],
        "description": "What to reindex"
      },
      "paths": {
        "type": "array",
        "items": { "type": "string" },
        "description": "File paths or directory paths (for scope=file/files/directory)"
      },
      "priority": {
        "type": "string",
        "enum": ["high", "normal", "low"],
        "default": "normal",
        "description": "Priority in queue"
      },
      "embeddings_only": {
        "type": "boolean",
        "default": false,
        "description": "Only regenerate embeddings, skip symbol extraction"
      },
      "async": {
        "type": "boolean",
        "default": true,
        "description": "Run in background (true) or block until complete (false)"
      },
      "incremental": {
        "type": "boolean",
        "default": false,
        "description": "Only reindex changed files (compare mtime)"
      }
    },
    "required": ["scope"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct ReindexingResponse {
    pub task_id: String,
    pub scope: ReindexScope,
    pub status: TaskStatus,
    pub items_total: usize,
    pub items_queued: usize,
    pub estimated_duration_seconds: u64,
    pub message: String,
    
    // For sync mode only
    pub result: Option<ReindexingResult>,
}

#[derive(Serialize)]
pub struct ReindexingResult {
    pub items_completed: usize,
    pub items_failed: usize,
    pub duration_seconds: u64,
    pub failures: Vec<FailedItem>,
    pub summary: ReindexingSummary,
}

#[derive(Serialize)]
pub struct FailedItem {
    pub path: String,
    pub error: String,
}

#[derive(Serialize)]
pub struct ReindexingSummary {
    pub files_processed: usize,
    pub symbols_extracted: usize,
    pub chunks_created: usize,
    pub embeddings_generated: usize,
}

#[derive(Serialize)]
pub enum ReindexScope {
    File { path: String },
    Files { paths: Vec<String> },
    Directory { path: String, recursive: bool },
    Project,
    EmbeddingsOnly,
}

#[derive(Serialize)]
pub enum TaskStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
}
```

### Example Response (Async)

```json
{
  "task_id": "reindex_20260216103000_abc123",
  "scope": {
    "Directory": {
      "path": "src/indexer",
      "recursive": true
    }
  },
  "status": "Queued",
  "items_total": 25,
  "items_queued": 25,
  "estimated_duration_seconds": 12,
  "message": "Reindexing task created. Use get_reindex_status(task_id) to track progress",
  "result": null
}
```

### Example Response (Sync, Completed)

```json
{
  "task_id": "reindex_20260216103015_def456",
  "scope": {
    "File": {
      "path": "src/main.rs"
    }
  },
  "status": "Completed",
  "items_total": 1,
  "items_queued": 0,
  "estimated_duration_seconds": 1,
  "message": "Reindexing completed successfully",
  "result": {
    "items_completed": 1,
    "items_failed": 0,
    "duration_seconds": 1,
    "failures": [],
    "summary": {
      "files_processed": 1,
      "symbols_extracted": 45,
      "chunks_created": 8,
      "embeddings_generated": 8
    }
  }
}
```

---

## 💻 Implementation Details

### Reindexing Manager

```rust
// src/indexer/reindex/manager.rs

use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;

pub struct ReindexingManager {
    sqlite: SqliteStorage,
    lance: LanceStorage,
    embedder: Embedder,
    parser: Parser,
    
    // Priority queue
    queue: Arc<RwLock<PriorityQueue<ReindexingTask>>>,
    
    // Channel for task control
    control_tx: mpsc::UnboundedSender<ControlMessage>,
    control_rx: Arc<Mutex<mpsc::UnboundedReceiver<ControlMessage>>>,
    
    // Current task
    current_task: Arc<RwLock<Option<ReindexingTask>>>,
}

impl ReindexingManager {
    pub fn new(
        sqlite: SqliteStorage,
        lance: LanceStorage,
        embedder: Embedder,
        parser: Parser,
    ) -> Self {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        
        Self {
            sqlite,
            lance,
            embedder,
            parser,
            queue: Arc::new(RwLock::new(PriorityQueue::new())),
            control_tx,
            control_rx: Arc::new(Mutex::new(control_rx)),
            current_task: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Start background worker
    pub async fn start(&self) {
        let manager = self.clone();
        
        tokio::spawn(async move {
            manager.worker_loop().await;
        });
    }
    
    /// Create reindexing task
    pub async fn create_task(
        &self,
        scope: ReindexScope,
        priority: Priority,
        options: ReindexOptions,
    ) -> Result<ReindexingResponse> {
        let task_id = self.generate_task_id();
        
        // Resolve paths based on scope
        let paths = self.resolve_paths(&scope).await?;
        
        // Filter if incremental
        let paths = if options.incremental {
            self.filter_changed_files(paths).await?
        } else {
            paths
        };
        
        // Create task record
        let task = ReindexingTask {
            id: task_id.clone(),
            scope: scope.clone(),
            paths: paths.clone(),
            priority,
            options,
            status: TaskStatus::Queued,
            progress: 0.0,
            items_total: paths.len(),
            items_completed: 0,
            items_failed: 0,
            error_message: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        
        // Insert to DB
        self.save_task(&task).await?;
        
        // Add to queue
        {
            let mut queue = self.queue.write().await;
            queue.push(task, priority);
        }
        
        // If async, return immediately
        if options.async_mode {
            Ok(ReindexingResponse {
                task_id: task_id.clone(),
                scope,
                status: TaskStatus::Queued,
                items_total: paths.len(),
                items_queued: paths.len(),
                estimated_duration_seconds: self.estimate_duration(paths.len()),
                message: "Reindexing task created. Use get_reindex_status(task_id) to track progress".into(),
                result: None,
            })
        } else {
            // Sync mode: wait for completion
            self.wait_for_task_completion(&task_id).await
        }
    }
    
    /// Background worker loop
    async fn worker_loop(&self) {
        loop {
            // Check for control messages
            if let Ok(msg) = self.control_rx.lock().await.try_recv() {
                self.handle_control_message(msg).await;
            }
            
            // Get next task from queue
            let task = {
                let mut queue = self.queue.write().await;
                queue.pop()
            };
            
            if let Some(task) = task {
                // Set as current task
                {
                    let mut current = self.current_task.write().await;
                    *current = Some(task.clone());
                }
                
                // Process task
                if let Err(e) = self.process_task(task).await {
                    error!("Task processing failed: {}", e);
                }
                
                // Clear current task
                {
                    let mut current = self.current_task.write().await;
                    *current = None;
                }
            } else {
                // No tasks, sleep
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    
    /// Process single reindexing task
    async fn process_task(&self, mut task: ReindexingTask) -> Result<()> {
        info!("Processing reindexing task: {}", task.id);
        
        // Update status
        task.status = TaskStatus::Running;
        task.started_at = Some(Utc::now());
        self.save_task(&task).await?;
        
        let start = Instant::now();
        
        // Process each file
        for (i, path) in task.paths.iter().enumerate() {
            // Check for pause/cancel
            if self.is_paused(&task.id).await {
                task.status = TaskStatus::Paused;
                self.save_task(&task).await?;
                return Ok(());
            }
            
            // Create task item
            let item_id = self.create_task_item(&task.id, path).await?;
            
            // Process file
            match self.reindex_file(path, &task.options).await {
                Ok(stats) => {
                    task.items_completed += 1;
                    self.complete_task_item(item_id, stats).await?;
                }
                Err(e) => {
                    error!("Failed to reindex {}: {}", path, e);
                    task.items_failed += 1;
                    self.fail_task_item(item_id, &e.to_string()).await?;
                }
            }
            
            // Update progress
            task.progress = (i + 1) as f32 / task.paths.len() as f32;
            self.save_task(&task).await?;
        }
        
        // Complete task
        task.status = if task.items_failed == 0 {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
        task.completed_at = Some(Utc::now());
        self.save_task(&task).await?;
        
        let duration = start.elapsed();
        info!(
            "Reindexing task {} completed in {:?}: {}/{} succeeded",
            task.id, duration, task.items_completed, task.paths.len()
        );
        
        Ok(())
    }
    
    /// Reindex single file
    async fn reindex_file(
        &self,
        path: &str,
        options: &ReindexOptions,
    ) -> Result<FileReindexStats> {
        info!("Reindexing file: {}", path);
        
        let mut stats = FileReindexStats::default();
        
        // Read file content
        let content = tokio::fs::read_to_string(path).await?;
        
        // Update file record
        let file_id = self.upsert_file(path, &content).await?;
        stats.files_processed = 1;
        
        // Skip symbol extraction if embeddings-only
        if !options.embeddings_only {
            // Parse and extract symbols
            let symbols = self.parser.parse(path, &content).await?;
            
            // Delete old symbols
            self.delete_old_symbols(file_id).await?;
            
            // Insert new symbols
            for symbol in symbols {
                self.sqlite.insert_symbol(&symbol).await?;
                stats.symbols_extracted += 1;
            }
        }
        
        // Chunk and embed
        let chunks = self.chunk_content(&content, file_id).await?;
        stats.chunks_created = chunks.len();
        
        // Delete old chunks
        self.delete_old_chunks(file_id).await?;
        
        // Insert new chunks
        for chunk in &chunks {
            self.sqlite.insert_chunk(chunk).await?;
        }
        
        // Generate embeddings
        let embeddings = self.embedder.embed_chunks(chunks).await?;
        stats.embeddings_generated = embeddings.len();
        
        // Delete old vectors from LanceDB
        self.lance.delete_vectors_for_file(file_id).await?;
        
        // Insert new vectors
        self.lance.insert_vectors(&embeddings).await?;
        
        // Update file status
        sqlx::query!(
            r#"
            UPDATE files
            SET 
                index_status = 'completed',
                indexed_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
            file_id
        )
        .execute(&self.sqlite.pool)
        .await?;
        
        Ok(stats)
    }
    
    async fn upsert_file(&self, path: &str, content: &str) -> Result<i64> {
        // Check if exists
        let existing = sqlx::query_scalar!(
            "SELECT id FROM files WHERE path = ?",
            path
        )
        .fetch_optional(&self.sqlite.pool)
        .await?;
        
        if let Some(id) = existing {
            // Update
            sqlx::query!(
                r#"
                UPDATE files
                SET 
                    content = ?,
                    updated_at = CURRENT_TIMESTAMP,
                    index_status = 'indexing'
                WHERE id = ?
                "#,
                content,
                id
            )
            .execute(&self.sqlite.pool)
            .await?;
            
            Ok(id)
        } else {
            // Insert
            let result = sqlx::query!(
                r#"
                INSERT INTO files (path, content, index_status)
                VALUES (?, ?, 'indexing')
                "#,
                path,
                content
            )
            .execute(&self.sqlite.pool)
            .await?;
            
            Ok(result.last_insert_rowid())
        }
    }
    
    async fn delete_old_symbols(&self, file_id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM symbols WHERE file_id = ?", file_id)
            .execute(&self.sqlite.pool)
            .await?;
        Ok(())
    }
    
    async fn delete_old_chunks(&self, file_id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM chunks WHERE file_id = ?", file_id)
            .execute(&self.sqlite.pool)
            .await?;
        Ok(())
    }
    
    async fn resolve_paths(&self, scope: &ReindexScope) -> Result<Vec<String>> {
        match scope {
            ReindexScope::File { path } => Ok(vec![path.clone()]),
            ReindexScope::Files { paths } => Ok(paths.clone()),
            ReindexScope::Directory { path, recursive } => {
                self.scan_directory(path, *recursive).await
            }
            ReindexScope::Project => {
                self.scan_workspace().await
            }
            ReindexScope::EmbeddingsOnly => {
                // All files currently indexed
                let paths = sqlx::query_scalar!("SELECT path FROM files WHERE index_status = 'completed'")
                    .fetch_all(&self.sqlite.pool)
                    .await?;
                Ok(paths)
            }
        }
    }
    
    async fn filter_changed_files(&self, paths: Vec<String>) -> Result<Vec<String>> {
        let mut changed = Vec::new();
        
        for path in paths {
            // Get file mtime
            let metadata = tokio::fs::metadata(&path).await?;
            let mtime = metadata.modified()?;
            
            // Get indexed_at from DB
            let indexed_at = sqlx::query_scalar!(
                "SELECT indexed_at FROM files WHERE path = ?",
                path
            )
            .fetch_optional(&self.sqlite.pool)
            .await?
            .flatten();
            
            // Compare
            if let Some(indexed_at) = indexed_at {
                let indexed_at: SystemTime = indexed_at.into();
                if mtime > indexed_at {
                    changed.push(path);
                }
            } else {
                // Not indexed yet
                changed.push(path);
            }
        }
        
        Ok(changed)
    }
    
    async fn scan_directory(&self, path: &str, recursive: bool) -> Result<Vec<String>> {
        // Use existing file watcher logic
        // (implementation details omitted)
        Ok(vec![])
    }
    
    async fn scan_workspace(&self) -> Result<Vec<String>> {
        // Use existing file watcher logic
        // (implementation details omitted)
        Ok(vec![])
    }
    
    fn generate_task_id(&self) -> String {
        format!("reindex_{}_{}", 
            Utc::now().format("%Y%m%d%H%M%S"),
            Uuid::new_v4().simple().to_string()[..8].to_string()
        )
    }
    
    fn estimate_duration(&self, file_count: usize) -> u64 {
        // Heuristic: ~0.5 seconds per file
        (file_count as f32 * 0.5).ceil() as u64
    }
    
    async fn save_task(&self, task: &ReindexingTask) -> Result<()> {
        let target_paths = serde_json::to_string(&task.paths)?;
        
        sqlx::query!(
            r#"
            INSERT OR REPLACE INTO reindexing_tasks
            (task_id, scope, target_paths, priority, status, progress, 
             items_total, items_completed, items_failed, error_message,
             created_at, started_at, completed_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            task.id,
            serde_json::to_string(&task.scope)?,
            target_paths,
            format!("{:?}", task.priority),
            format!("{:?}", task.status),
            task.progress,
            task.items_total,
            task.items_completed,
            task.items_failed,
            task.error_message,
            task.created_at,
            task.started_at,
            task.completed_at,
        )
        .execute(&self.sqlite.pool)
        .await?;
        
        Ok(())
    }
    
    async fn create_task_item(&self, task_id: &str, path: &str) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            INSERT INTO reindexing_task_items (task_id, file_path, status)
            VALUES (?, ?, 'pending')
            "#,
            task_id,
            path
        )
        .execute(&self.sqlite.pool)
        .await?;
        
        Ok(result.last_insert_rowid())
    }
    
    async fn complete_task_item(&self, item_id: i64, stats: FileReindexStats) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE reindexing_task_items
            SET 
                status = 'completed',
                completed_at = CURRENT_TIMESTAMP,
                duration_ms = ?
            WHERE id = ?
            "#,
            stats.duration_ms,
            item_id
        )
        .execute(&self.sqlite.pool)
        .await?;
        
        Ok(())
    }
    
    async fn fail_task_item(&self, item_id: i64, error: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE reindexing_task_items
            SET 
                status = 'failed',
                error_message = ?,
                completed_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
            error,
            item_id
        )
        .execute(&self.sqlite.pool)
        .await?;
        
        Ok(())
    }
    
    async fn wait_for_task_completion(&self, task_id: &str) -> Result<ReindexingResponse> {
        // Poll task status
        loop {
            let task = self.get_task(task_id).await?;
            
            match task.status {
                TaskStatus::Completed | TaskStatus::Failed => {
                    return self.build_response_with_result(task).await;
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
    
    async fn build_response_with_result(&self, task: ReindexingTask) -> Result<ReindexingResponse> {
        let failures = self.get_task_failures(&task.id).await?;
        let summary = self.get_task_summary(&task.id).await?;
        let duration = task.completed_at.unwrap() - task.started_at.unwrap();
        
        Ok(ReindexingResponse {
            task_id: task.id,
            scope: task.scope,
            status: task.status,
            items_total: task.items_total,
            items_queued: 0,
            estimated_duration_seconds: 0,
            message: "Reindexing completed".into(),
            result: Some(ReindexingResult {
                items_completed: task.items_completed,
                items_failed: task.items_failed,
                duration_seconds: duration.num_seconds() as u64,
                failures,
                summary,
            }),
        })
    }
}
```

### Additional Tools

```rust
// Get task status
pub async fn get_reindex_status(task_id: &str) -> Result<TaskStatus> {
    // Query database for task status
}

// Cancel task
pub async fn cancel_reindex(task_id: &str) -> Result<()> {
    // Send cancel message to worker
}

// Pause task
pub async fn pause_reindex(task_id: &str) -> Result<()> {
    // Send pause message to worker
}

// Resume task
pub async fn resume_reindex(task_id: &str) -> Result<()> {
    // Send resume message to worker
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_reindex_single_file() {
    let manager = setup_test_manager().await;
    
    let response = manager.create_task(
        ReindexScope::File { path: "test.rs".into() },
        Priority::Normal,
        ReindexOptions { async_mode: false, ..Default::default() },
    ).await.unwrap();
    
    assert_eq!(response.status, TaskStatus::Completed);
    let result = response.result.unwrap();
    assert_eq!(result.items_completed, 1);
    assert_eq!(result.items_failed, 0);
}

#[tokio::test]
async fn test_incremental_reindex() {
    let manager = setup_test_manager().await;
    
    // Initial index
    manager.create_task(
        ReindexScope::Project,
        Priority::Normal,
        ReindexOptions::default(),
    ).await.unwrap();
    
    // Modify one file
    tokio::fs::write("test.rs", "// modified").await.unwrap();
    
    // Incremental reindex
    let response = manager.create_task(
        ReindexScope::Project,
        Priority::Normal,
        ReindexOptions { incremental: true, async_mode: false, ..Default::default() },
    ).await.unwrap();
    
    // Should only reindex 1 file
    assert_eq!(response.items_total, 1);
}
```

---

## 📈 Success Metrics

- ✅ Reindexing works without daemon restart
- ✅ Progress tracking is accurate (±5%)
- ⏱️ Incremental reindex is 10× faster than full
- ✅ No queries fail during reindexing
- ✅ Failed files don't block queue

---

## ✅ Acceptance Criteria

- [ ] Single file reindexing works
- [ ] Directory reindexing works (recursive)
- [ ] Full project reindexing works
- [ ] Embeddings-only mode works
- [ ] Incremental mode only processes changed files
- [ ] Progress tracking is accurate
- [ ] Async mode returns immediately
- [ ] Sync mode blocks until complete
- [ ] Failed files are logged properly
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
