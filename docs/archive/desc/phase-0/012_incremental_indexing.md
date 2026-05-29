# Feature: incremental_indexing - Инкрементальная индексация

**ID:** PHASE0-012  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)

---

## 📋 Описание

Система инкрементального обновления индекса при изменении файлов. Вместо полной переиндексации проекта, обновляются только измененные файлы. Критично для производительности в больших проектах.

### Проблема

**Текущее поведение (full reindex):**
```
Developer: меняет 1 файл (10 строк)

gofer:
1. Обнаруживает изменение
2. Запускает ПОЛНУЮ переиндексацию
   - 1000 файлов
   - 100,000 строк кода
   - 10,000 symbols
   - 5000 embeddings
3. Занимает: 2-3 минуты ⏱️

Developer: 😴 ждет...
```

**Проблемы:**
- Огромный waste of resources (99.9% кода не изменился)
- Долгое время индексации
- Высокая нагрузка на CPU/embedding API
- Плохой UX (долго ждать после каждого save)

### Решение

**С incremental indexing:**
```
Developer: меняет 1 файл (10 строк)

gofer:
1. Обнаруживает изменение в file.rs
2. Инкрементальное обновление:
   ✓ Re-parse только file.rs
   ✓ Update только symbols из file.rs
   ✓ Re-embed только измененные chunks
   ✓ Invalidate affected caches
3. Занимает: 2-3 секунды ⚡

Developer: 🚀 продолжает работать

Speedup: 60× быстрее! (180s → 3s)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Обновлять только измененные файлы
- ✅ 50-100× faster vs full reindex
- ✅ Поддержка file create/update/delete/rename
- ✅ Корректная обработка зависимостей
- ✅ Transactional updates (atomic)
- ✅ Background processing

### Non-Goals
- ❌ Не обрабатывает изменения вне проекта
- ❌ Не гарантирует мгновенное обновление (есть latency)
- ❌ Не обрабатывает конфликты версий файлов

---

## 🏗️ Архитектура

```
┌─────────────────────────────────────────┐
│         File Watcher                    │
│      (notify crate)                     │
└────────────────┬────────────────────────┘
                 │ file change event
        ┌────────▼────────┐
        │ Change Detector │
        │  (debounce)     │
        └────────┬────────┘
                 │
        ┌────────▼────────┐
        │ Change Analyzer │
        │ (what changed?) │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬────────────┐
     │           │           │            │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐ ┌───▼────┐
│  Parse   │ │Update│ │Re-generate│ │Invalidate│
│   File   │ │Symbols│ │Embeddings │ │  Cache  │
└──────────┘ └──────┘ └───────────┘ └────────┘
                 │
        ┌────────▼────────┐
        │  Transaction    │
        │   Commit        │
        └─────────────────┘
```

### Change Detection Flow

```
1. File Watcher detects change
   ↓
2. Debounce (wait 500ms for more changes)
   ↓
3. Analyze change type:
   ├─→ Created: full index new file
   ├─→ Modified: re-index file
   ├─→ Deleted: remove from index
   └─→ Renamed: update path references
   ↓
4. Load old file metadata
   ↓
5. Parse new file content
   ↓
6. Diff symbols (added/removed/changed)
   ↓
7. Update database (in transaction):
   ├─→ Delete old symbols
   ├─→ Insert new symbols
   ├─→ Update file metadata
   └─→ Re-generate embeddings
   ↓
8. Invalidate affected caches
   ↓
9. Commit transaction
```

---

## 📊 Data Model

### Change Event

```rust
#[derive(Debug)]
pub enum FileChangeEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

#[derive(Debug)]
pub struct ChangeAnalysis {
    pub event: FileChangeEvent,
    pub old_metadata: Option<FileMetadata>,
    pub new_content: Option<String>,
    pub affected_symbols: Vec<i64>,  // Symbol IDs to update
    pub affected_files: Vec<String>,  // Other files that reference this
}
```

### File Metadata

```sql
-- Расширение таблицы files для tracking changes
ALTER TABLE files ADD COLUMN last_indexed_at DATETIME;
ALTER TABLE files ADD COLUMN content_hash TEXT;  -- SHA256 of content
ALTER TABLE files ADD COLUMN mtime INTEGER;      -- File modification time

CREATE INDEX idx_files_mtime ON files(mtime);
CREATE INDEX idx_files_content_hash ON files(content_hash);
```

---

## 💻 Implementation Details

### Step 1: File Watcher

```rust
// src/watcher/incremental.rs

use notify::{Watcher, RecursiveMode, Event};
use tokio::sync::mpsc;

pub struct IncrementalWatcher {
    watcher: RecommendedWatcher,
    event_rx: mpsc::Receiver<FileChangeEvent>,
    debouncer: Debouncer,
}

impl IncrementalWatcher {
    pub fn new(project_root: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::channel(1000);
        
        let watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                let change = FileChangeEvent::from_notify_event(event);
                let _ = tx.blocking_send(change);
            }
        })?;
        
        watcher.watch(&project_root, RecursiveMode::Recursive)?;
        
        Ok(Self {
            watcher,
            event_rx: rx,
            debouncer: Debouncer::new(Duration::from_millis(500)),
        })
    }
    
    pub async fn watch(&mut self) -> Result<()> {
        while let Some(event) = self.event_rx.recv().await {
            // Debounce: collect multiple rapid changes
            self.debouncer.push(event);
            
            if let Some(batch) = self.debouncer.try_flush() {
                self.process_batch(batch).await?;
            }
        }
        
        Ok(())
    }
    
    async fn process_batch(&self, events: Vec<FileChangeEvent>) -> Result<()> {
        // Group events by file
        let grouped = self.group_events(events);
        
        for (path, event) in grouped {
            self.process_single_change(event).await?;
        }
        
        Ok(())
    }
}
```

### Step 2: Change Analyzer

```rust
// src/indexer/change_analyzer.rs

pub struct ChangeAnalyzer {
    sqlite: SqliteStorage,
}

impl ChangeAnalyzer {
    pub async fn analyze_change(
        &self,
        event: FileChangeEvent
    ) -> Result<ChangeAnalysis> {
        match event {
            FileChangeEvent::Created(path) => {
                self.analyze_created(&path).await
            }
            
            FileChangeEvent::Modified(path) => {
                self.analyze_modified(&path).await
            }
            
            FileChangeEvent::Deleted(path) => {
                self.analyze_deleted(&path).await
            }
            
            FileChangeEvent::Renamed { from, to } => {
                self.analyze_renamed(&from, &to).await
            }
        }
    }
    
    async fn analyze_modified(&self, path: &Path) -> Result<ChangeAnalysis> {
        // 1. Load old metadata
        let old_metadata = self.sqlite.get_file_metadata(path).await?;
        
        // 2. Read new content
        let new_content = tokio::fs::read_to_string(path).await?;
        
        // 3. Check if actually changed (content hash)
        let new_hash = sha256(&new_content);
        
        if let Some(ref old_meta) = old_metadata {
            if old_meta.content_hash == new_hash {
                // No actual change (maybe just mtime changed)
                return Ok(ChangeAnalysis {
                    event: FileChangeEvent::Modified(path.to_path_buf()),
                    old_metadata,
                    new_content: None,
                    affected_symbols: vec![],
                    affected_files: vec![],
                });
            }
        }
        
        // 4. Find affected symbols
        let affected_symbols = if let Some(ref old_meta) = old_metadata {
            self.sqlite.get_symbols_for_file(old_meta.id).await?
        } else {
            vec![]
        };
        
        // 5. Find files that reference this file
        let affected_files = self.find_dependent_files(path).await?;
        
        Ok(ChangeAnalysis {
            event: FileChangeEvent::Modified(path.to_path_buf()),
            old_metadata,
            new_content: Some(new_content),
            affected_symbols: affected_symbols.into_iter().map(|s| s.id).collect(),
            affected_files,
        })
    }
    
    async fn find_dependent_files(&self, path: &Path) -> Result<Vec<String>> {
        // Find files that import/reference this file
        // Query symbols that reference symbols from this file
        
        let path_str = path.to_str().unwrap();
        
        sqlx::query_scalar!(
            r#"
            SELECT DISTINCT f.path
            FROM files f
            JOIN symbols s ON s.file_id = f.id
            WHERE s.definition_file = ?
            "#,
            path_str
        )
        .fetch_all(&self.sqlite.pool)
        .await
        .map_err(Into::into)
    }
}
```

### Step 3: Incremental Indexer

```rust
// src/indexer/incremental.rs

pub struct IncrementalIndexer {
    sqlite: SqliteStorage,
    lance: LanceStorage,
    parser: LanguageParser,
    embedder: Embedder,
    cache: CacheManager,
}

impl IncrementalIndexer {
    pub async fn update_file(&self, analysis: ChangeAnalysis) -> Result<()> {
        let path = match &analysis.event {
            FileChangeEvent::Modified(p) | FileChangeEvent::Created(p) => p,
            FileChangeEvent::Deleted(p) => {
                return self.delete_file(p).await;
            }
            FileChangeEvent::Renamed { from, to } => {
                return self.rename_file(from, to).await;
            }
        };
        
        let content = analysis.new_content
            .ok_or_else(|| anyhow!("No content for update"))?;
        
        // Start transaction
        let mut tx = self.sqlite.pool.begin().await?;
        
        // 1. Delete old symbols
        if !analysis.affected_symbols.is_empty() {
            sqlx::query!(
                "DELETE FROM symbols WHERE id IN (?)",
                analysis.affected_symbols
            )
            .execute(&mut *tx)
            .await?;
        }
        
        // 2. Parse new content
        let parse_result = self.parser.parse(path, &content)?;
        
        // 3. Insert/update file metadata
        let file_id = self.upsert_file_metadata(
            &mut tx,
            path,
            &content,
            &parse_result
        ).await?;
        
        // 4. Insert new symbols
        for symbol in parse_result.symbols {
            self.insert_symbol(&mut tx, file_id, symbol).await?;
        }
        
        // 5. Generate chunks
        let chunks = self.generate_chunks(&content, &parse_result)?;
        
        // 6. Delete old embeddings
        self.lance.delete_vectors_for_file(path).await?;
        
        // 7. Generate new embeddings
        for chunk in chunks {
            let embedding = self.embedder.embed(&chunk.text).await?;
            self.lance.insert_vector(chunk.id, embedding).await?;
        }
        
        // 8. Invalidate caches
        self.cache.invalidate_file(path.to_str().unwrap()).await;
        if let Some(old_meta) = analysis.old_metadata {
            self.cache.invalidate_symbols(old_meta.id).await;
        }
        
        // Commit transaction
        tx.commit().await?;
        
        info!("Incrementally updated file: {}", path.display());
        
        Ok(())
    }
    
    async fn upsert_file_metadata(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        path: &Path,
        content: &str,
        parse_result: &ParseResult,
    ) -> Result<i64> {
        let path_str = path.to_str().unwrap();
        let content_hash = sha256(content);
        let mtime = std::fs::metadata(path)?
            .modified()?
            .duration_since(UNIX_EPOCH)?
            .as_secs() as i64;
        
        let file_id = sqlx::query_scalar!(
            r#"
            INSERT INTO files (path, content_hash, mtime, last_indexed_at)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(path) DO UPDATE SET
                content_hash = excluded.content_hash,
                mtime = excluded.mtime,
                last_indexed_at = CURRENT_TIMESTAMP
            RETURNING id
            "#,
            path_str,
            content_hash,
            mtime
        )
        .fetch_one(&mut **tx)
        .await?;
        
        Ok(file_id)
    }
    
    async fn delete_file(&self, path: &Path) -> Result<()> {
        let path_str = path.to_str().unwrap();
        
        // Transaction
        let mut tx = self.sqlite.pool.begin().await?;
        
        // Delete from SQLite
        sqlx::query!("DELETE FROM symbols WHERE file = ?", path_str)
            .execute(&mut *tx)
            .await?;
        
        sqlx::query!("DELETE FROM files WHERE path = ?", path_str)
            .execute(&mut *tx)
            .await?;
        
        // Delete from LanceDB
        self.lance.delete_vectors_for_file(path).await?;
        
        // Invalidate cache
        self.cache.invalidate_file(path_str).await;
        
        tx.commit().await?;
        
        info!("Deleted file from index: {}", path.display());
        
        Ok(())
    }
}
```

---

## 🧪 Testing

```rust
#[tokio::test]
async fn test_incremental_update_single_file() {
    let (indexer, temp_dir) = setup_test_indexer().await;
    
    // Initial index
    let test_file = temp_dir.path().join("test.rs");
    tokio::fs::write(&test_file, "fn foo() {}").await.unwrap();
    indexer.index_file(&test_file).await.unwrap();
    
    // Verify initial state
    let symbols = indexer.get_symbols(&test_file).await.unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "foo");
    
    // Modify file
    tokio::fs::write(&test_file, "fn foo() {} fn bar() {}").await.unwrap();
    
    // Incremental update
    let start = Instant::now();
    indexer.update_file(&test_file).await.unwrap();
    let elapsed = start.elapsed();
    
    // Verify update
    let symbols = indexer.get_symbols(&test_file).await.unwrap();
    assert_eq!(symbols.len(), 2);
    assert!(symbols.iter().any(|s| s.name == "foo"));
    assert!(symbols.iter().any(|s| s.name == "bar"));
    
    // Check performance
    assert!(elapsed < Duration::from_secs(5), "Incremental update too slow");
}
```

---

## 📈 Success Metrics

- ⚡ 50-100× faster than full reindex
- ⏱️ < 5s для файла < 1000 строк
- ✅ 100% correctness (no lost data)
- 🔄 Handles 100+ file changes/minute

---

## ✅ Acceptance Criteria

- [ ] Detects file changes automatically
- [ ] Updates only changed files
- [ ] 50× faster than full reindex
- [ ] Handles create/update/delete/rename
- [ ] Transactional updates
- [ ] Cache invalidation works
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** КРИТИЧЕСКИЙ - делает gofer пригодным для больших проектов.
