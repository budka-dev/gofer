# Phase 0: Foundation & Quick Wins - Implementation Plan

**Status:** Ready to Start  
**Date:** 2026-02-16  
**Priority:** 🔥🔥🔥🔥 CRITICAL  
**Duration:** 2-4 weeks  
**Selected Features:** 001-003 (Index Quality), 004-010 (Token Efficiency), 012-016 (Performance)

---

## 📊 Executive Summary

Phase 0 establishes the critical foundation for gofer MCP with focus on:
- **Index Quality & Visibility** - understand what's indexed
- **Token Efficiency** - 50-95% savings through smart reading
- **Performance Infrastructure** - caching, pooling, optimization

### Expected Impact
- ✅ 50-70% token savings immediately
- ✅ Reliable index with full visibility
- ✅ 30-40% cache hit rate
- ✅ 50-100× faster incremental indexing

---

## 🎯 Implementation Phases

### Phase 0.1: Index Quality & Visibility (3 days)
**Goal:** Full transparency into index state

#### Features
1. **001_get_index_status** (1 day)
   - Add `index_metadata` table
   - Implement MCP tool `get_index_status`
   - Show completeness, last sync, queue status
   
2. **002_validate_index** (1 day)
   - Implement gap detection
   - Check consistency (files vs symbols vs embeddings)
   - Report issues with actionable fixes
   
3. **003_force_reindex** (1 day)
   - Add priority queue to watcher
   - Implement `force_reindex` tool
   - Support file/directory/full project modes

#### Implementation Steps

**Step 1: Database Schema** (2 hours)
```sql
-- migrations/013_index_metadata.sql
CREATE TABLE IF NOT EXISTS index_metadata (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Track indexing state
INSERT INTO index_metadata (key, value) VALUES 
    ('last_full_sync', ''),
    ('indexing_started_at', ''),
    ('indexing_completed_at', ''),
    ('total_files_indexed', '0'),
    ('total_symbols_indexed', '0'),
    ('total_chunks_indexed', '0');

-- Add tracking fields to files table
ALTER TABLE files ADD COLUMN last_indexed_at DATETIME;
ALTER TABLE files ADD COLUMN content_hash TEXT;
ALTER TABLE files ADD COLUMN indexing_status TEXT DEFAULT 'pending'; -- pending, indexing, completed, failed

CREATE INDEX idx_files_indexing_status ON files(indexing_status);
CREATE INDEX idx_files_last_indexed ON files(last_indexed_at);
```

**Step 2: Index Status Tool** (4 hours)
```rust
// src/daemon/tools.rs - Add to dispatch()
"get_index_status" => tool_get_index_status(ctx).await,

// Implement tool
async fn tool_get_index_status(ctx: &ToolContext<'_>) -> Result<Value> {
    let sqlite = ctx.sqlite;
    
    // Get counts
    let file_count = sqlx::query_scalar!("SELECT COUNT(*) FROM files")
        .fetch_one(&sqlite.pool).await?;
    let symbol_count = sqlx::query_scalar!("SELECT COUNT(*) FROM symbols")
        .fetch_one(&sqlite.pool).await?;
    let chunk_count = sqlx::query_scalar!("SELECT COUNT(*) FROM chunks")
        .fetch_one(&sqlite.pool).await?;
    
    // Get embedding count from LanceDB
    let lance = ctx.lance.lock().await;
    let embedding_count = lance.count_vectors().await?;
    
    // Get metadata
    let last_sync = sqlx::query_scalar!(
        "SELECT value FROM index_metadata WHERE key = 'last_full_sync'"
    ).fetch_one(&sqlite.pool).await?;
    
    // Calculate completeness
    let pending_files = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM files WHERE indexing_status = 'pending'"
    ).fetch_one(&sqlite.pool).await?;
    
    let completeness = if file_count > 0 {
        ((file_count - pending_files) as f64 / file_count as f64 * 100.0)
    } else { 0.0 };
    
    Ok(json!({
        "files": { "total": file_count, "pending": pending_files },
        "symbols": symbol_count,
        "chunks": chunk_count,
        "embeddings": embedding_count,
        "last_sync": last_sync,
        "completeness_percent": completeness,
        "status": if pending_files == 0 { "complete" } else { "indexing" }
    }))
}
```

**Step 3: Validate Index Tool** (4 hours)
```rust
// src/daemon/tools.rs
"validate_index" => tool_validate_index(ctx).await,

async fn tool_validate_index(ctx: &ToolContext<'_>) -> Result<Value> {
    let mut issues = Vec::new();
    
    // Check 1: Files without symbols
    let files_without_symbols = sqlx::query!(
        r#"
        SELECT f.path, f.language
        FROM files f
        LEFT JOIN symbols s ON s.file_id = f.id
        WHERE s.id IS NULL
        AND f.language IN ('rust', 'typescript', 'python', 'go')
        "#
    ).fetch_all(&ctx.sqlite.pool).await?;
    
    if !files_without_symbols.is_empty() {
        issues.push(json!({
            "type": "missing_symbols",
            "severity": "warning",
            "count": files_without_symbols.len(),
            "message": "Files indexed without symbols extracted",
            "files": files_without_symbols.iter()
                .take(10)
                .map(|r| r.path.clone())
                .collect::<Vec<_>>()
        }));
    }
    
    // Check 2: Symbols without files (orphaned)
    let orphaned_symbols = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM symbols s
        LEFT JOIN files f ON s.file_id = f.id
        WHERE f.id IS NULL
        "#
    ).fetch_one(&ctx.sqlite.pool).await?;
    
    if orphaned_symbols > 0 {
        issues.push(json!({
            "type": "orphaned_symbols",
            "severity": "error",
            "count": orphaned_symbols,
            "message": "Symbols without corresponding files"
        }));
    }
    
    // Check 3: Files vs embeddings mismatch
    let lance = ctx.lance.lock().await;
    let file_count = sqlx::query_scalar!("SELECT COUNT(*) FROM files")
        .fetch_one(&ctx.sqlite.pool).await?;
    let embedding_files = lance.count_unique_files().await?;
    
    if file_count > 0 && (embedding_files as f64 / file_count as f64) < 0.9 {
        issues.push(json!({
            "type": "missing_embeddings",
            "severity": "warning",
            "message": "Significant mismatch between files and embeddings",
            "files_count": file_count,
            "embedding_files_count": embedding_files
        }));
    }
    
    Ok(json!({
        "valid": issues.is_empty(),
        "issues": issues,
        "recommendation": if !issues.is_empty() {
            "Run force_reindex to fix issues"
        } else {
            "Index is healthy"
        }
    }))
}
```

**Step 4: Force Reindex Tool** (4 hours)
```rust
// src/indexer/watcher.rs - Add priority queue
pub struct IndexingQueue {
    high_priority: Vec<PathBuf>,
    normal_priority: Vec<PathBuf>,
}

// src/daemon/tools.rs
"force_reindex" => tool_force_reindex(args, ctx).await,

async fn tool_force_reindex(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let path = args.get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    
    let scope = args.get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("project");
    
    match scope {
        "file" => {
            let path = path.ok_or_else(|| anyhow!("path required for file scope"))?;
            // Queue single file with high priority
            queue_file_reindex(&path, true).await?;
            Ok(json!({ "status": "queued", "files": 1 }))
        }
        "directory" => {
            let dir = path.ok_or_else(|| anyhow!("path required for directory scope"))?;
            let files = collect_files_in_dir(&dir)?;
            for file in &files {
                queue_file_reindex(file, true).await?;
            }
            Ok(json!({ "status": "queued", "files": files.len() }))
        }
        "project" => {
            // Mark all files as pending
            sqlx::query!(
                "UPDATE files SET indexing_status = 'pending'"
            ).execute(&ctx.sqlite.pool).await?;
            
            Ok(json!({ "status": "full_reindex_queued" }))
        }
        _ => Err(anyhow!("Invalid scope"))
    }
}
```

---

### Phase 0.2: Token Efficiency - Quick Wins (2 days)
**Goal:** Immediate 3-5× token savings

#### Features
4. **004_read_file_skeleton** (0.5 day)
   - ✅ Already implemented in `skeleton.rs`
   - Add MCP tool wrapper
   
5. **005_lightweight_checks** (1 day)
   - Implement `file_exists`, `symbol_exists`, `has_tests_for`
   - 95% token savings vs full read
   
6. **006_search_with_scores** (0.5 day)
   - Add confidence scores to search results
   - Implement preview mode (snippets only)

#### Implementation Steps

**Step 1: Skeleton Tool** (2 hours)
```rust
// src/daemon/tools.rs - Already exists, ensure it's exposed
// Just verify tool_skeleton() is working correctly

async fn tool_skeleton(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let file_path = args.get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("file parameter required"))?;
    
    let full_path = ctx.root_path.join(file_path);
    let content = tokio::fs::read_to_string(&full_path).await?;
    
    // Detect language
    let language = detect_language_from_path(&full_path)?;
    
    // Generate skeleton
    let skeleton = crate::indexer::parser::skeleton::generate_skeleton(
        &content, 
        language
    )?;
    
    Ok(json!({
        "file": file_path,
        "skeleton": skeleton,
        "original_lines": content.lines().count(),
        "skeleton_lines": skeleton.lines().count(),
        "savings_percent": calculate_token_savings(&content, &skeleton)
    }))
}
```

**Step 2: Lightweight Checks** (4 hours)
```rust
// src/daemon/tools.rs
"file_exists" => tool_file_exists(args, ctx).await,
"symbol_exists" => tool_symbol_exists(args, ctx).await,
"has_tests_for" => tool_has_tests_for(args, ctx).await,

async fn tool_file_exists(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let path = args.get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("path required"))?;
    
    let exists = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM files WHERE path = ?",
        path
    ).fetch_one(&ctx.sqlite.pool).await? > 0;
    
    Ok(json!({ "exists": exists, "path": path }))
}

async fn tool_symbol_exists(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let symbol = args.get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("symbol required"))?;
    
    let file = args.get("file").and_then(|v| v.as_str());
    
    let exists = if let Some(file_path) = file {
        sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) 
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE s.name = ? AND f.path = ?
            "#,
            symbol, file_path
        ).fetch_one(&ctx.sqlite.pool).await? > 0
    } else {
        sqlx::query_scalar!(
            "SELECT COUNT(*) FROM symbols WHERE name = ?",
            symbol
        ).fetch_one(&ctx.sqlite.pool).await? > 0
    };
    
    Ok(json!({ "exists": exists, "symbol": symbol }))
}

async fn tool_has_tests_for(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let file_path = args.get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("file required"))?;
    
    // Check for test files following common patterns
    let test_patterns = vec![
        format!("{}.test.ts", file_path.trim_end_matches(".ts")),
        format!("{}.spec.ts", file_path.trim_end_matches(".ts")),
        format!("tests/test_{}", file_path),
        format!("tests/{}_test.rs", file_path.trim_end_matches(".rs")),
    ];
    
    for pattern in test_patterns {
        let exists = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM files WHERE path LIKE ?",
            format!("%{}%", pattern)
        ).fetch_one(&ctx.sqlite.pool).await? > 0;
        
        if exists {
            return Ok(json!({ "has_tests": true, "test_file": pattern }));
        }
    }
    
    Ok(json!({ "has_tests": false }))
}
```

**Step 3: Search with Scores** (2 hours)
```rust
// Enhance existing tool_search to include confidence scores
async fn tool_search(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    // ... existing search logic ...
    
    let preview_mode = args.get("preview")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    // Add confidence calculation
    let results_with_scores: Vec<_> = results.into_iter().map(|r| {
        let confidence = calculate_confidence_score(&r);
        
        json!({
            "file": r.file,
            "line": r.line,
            "content": if preview_mode {
                truncate_to_snippet(&r.content, 150)
            } else {
                r.content
            },
            "score": r.score,
            "confidence": confidence,
            "reason": explain_confidence(confidence)
        })
    }).collect();
    
    Ok(json!({ "results": results_with_scores }))
}

fn calculate_confidence_score(result: &SearchResult) -> f32 {
    // Multi-factor confidence:
    // - Semantic similarity (40%)
    // - Keyword match (30%)
    // - File recency (15%)
    // - Symbol type match (15%)
    
    let semantic_score = result.score; // 0-1
    let keyword_bonus = if has_exact_keyword_match(result) { 0.3 } else { 0.0 };
    let recency_score = calculate_recency_score(result.last_modified);
    
    (semantic_score * 0.4 + keyword_bonus + recency_score * 0.15).min(1.0)
}
```

---

### Phase 0.3: Performance Infrastructure (3 days)
**Goal:** Connection pooling and error recovery

#### Features
15. **015_connection_pooling** (1 day)
   - Already using sqlx connection pool ✅
   - Optimize pool settings
   - Add pool metrics
   
16. **016_error_recovery** (2 days)
   - Circuit breaker for embeddings
   - Graceful degradation
   - Retry logic with backoff

#### Implementation Steps

**Step 1: Optimize Pool Settings** (2 hours)
```rust
// src/storage/sqlite.rs - Update pool configuration
impl SqliteStorage {
    pub async fn new(db_path: &Path) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(10)  // Increase from default
            .min_connections(2)   // Keep 2 warm connections
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(300))
            .max_lifetime(Duration::from_secs(1800))
            .connect(&format!("sqlite:{}", db_path.display()))
            .await?;
        
        // Enable WAL mode for better concurrency
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool).await?;
        
        Ok(Self { pool })
    }
}
```

**Step 2: Circuit Breaker** (6 hours)
```rust
// src/indexer/circuit_breaker.rs
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct CircuitBreaker {
    failure_count: AtomicU32,
    last_failure: AtomicU64,
    threshold: u32,
    timeout: Duration,
}

#[derive(Debug, PartialEq)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failing, reject requests
    HalfOpen, // Testing if service recovered
}

impl CircuitBreaker {
    pub fn new(threshold: u32, timeout: Duration) -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            last_failure: AtomicU64::new(0),
            threshold,
            timeout,
        }
    }
    
    pub fn state(&self) -> CircuitState {
        let failures = self.failure_count.load(Ordering::Relaxed);
        
        if failures < self.threshold {
            return CircuitState::Closed;
        }
        
        let last_fail_nanos = self.last_failure.load(Ordering::Relaxed);
        let elapsed = Duration::from_nanos(
            Instant::now().elapsed().as_nanos() as u64 - last_fail_nanos
        );
        
        if elapsed > self.timeout {
            CircuitState::HalfOpen
        } else {
            CircuitState::Open
        }
    }
    
    pub fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
    }
    
    pub fn record_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.last_failure.store(
            Instant::now().elapsed().as_nanos() as u64,
            Ordering::Relaxed
        );
    }
    
    pub async fn call<F, T, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce() -> Result<T, E>,
        E: std::fmt::Display,
    {
        match self.state() {
            CircuitState::Open => {
                return Err(format!("Circuit breaker open").into());
            }
            CircuitState::HalfOpen | CircuitState::Closed => {
                match f() {
                    Ok(result) => {
                        self.record_success();
                        Ok(result)
                    }
                    Err(e) => {
                        self.record_failure();
                        Err(e)
                    }
                }
            }
        }
    }
}

// Apply to embedder
// src/indexer/embedder.rs
pub struct EmbedderWithCircuitBreaker {
    embedder: EmbedderPool,
    breaker: Arc<CircuitBreaker>,
}

impl EmbedderWithCircuitBreaker {
    pub async fn embed_with_retry(&self, text: &str) -> Result<Vec<f32>> {
        let mut retries = 0;
        let max_retries = 3;
        
        loop {
            match self.breaker.call(|| self.embedder.embed(text)).await {
                Ok(embedding) => return Ok(embedding),
                Err(e) if retries < max_retries => {
                    retries += 1;
                    let backoff = Duration::from_millis(100 * 2_u64.pow(retries));
                    tracing::warn!(
                        "Embedding failed (attempt {}/{}), retrying in {:?}: {}",
                        retries, max_retries, backoff, e
                    );
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
```

---

### Phase 0.4: Advanced Token Efficiency (2 days)
**Goal:** Function-level and type-only reading

#### Features
9. **009_read_function_context** (1 day)
   - Extract single function with dependencies
   - 90-95% token savings
   
10. **010_read_types_only** (1 day)
   - Extract only type definitions
   - Perfect for data model analysis

#### Implementation Steps

**Step 1: Function Context Extractor** (6 hours)
```rust
// src/indexer/parser/function_context.rs
pub struct FunctionContext {
    pub function: String,  // Function signature + body
    pub imports: Vec<String>,
    pub types: Vec<String>,  // Referenced types
    pub dependencies: Vec<String>,  // Other functions called
}

pub fn extract_function_context(
    code: &str,
    function_name: &str,
    language: SupportedLanguage
) -> Result<FunctionContext> {
    let mut parser = Parser::new();
    parser.set_language(&language.tree_sitter_language())?;
    
    let tree = parser.parse(code, None).ok_or(ParserError::ParseError)?;
    let root = tree.root_node();
    
    // Find target function
    let function_node = find_function_by_name(root, function_name, code)?;
    
    // Extract function text
    let function_text = code[function_node.start_byte()..function_node.end_byte()].to_string();
    
    // Find imports used by this function
    let imports = extract_imports_for_function(&function_node, root, code);
    
    // Find type definitions referenced
    let types = extract_referenced_types(&function_node, code);
    
    // Find function calls
    let dependencies = extract_function_calls(&function_node, code);
    
    Ok(FunctionContext {
        function: function_text,
        imports,
        types,
        dependencies,
    })
}

// MCP tool
async fn tool_read_function_context(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let file_path = args.get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("file required"))?;
    
    let function_name = args.get("function")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("function required"))?;
    
    let full_path = ctx.root_path.join(file_path);
    let content = tokio::fs::read_to_string(&full_path).await?;
    let language = detect_language_from_path(&full_path)?;
    
    let context = extract_function_context(&content, function_name, language)?;
    
    Ok(json!({
        "function": context.function,
        "imports": context.imports,
        "types": context.types,
        "dependencies": context.dependencies,
        "savings_percent": 90.0  // Approximate
    }))
}
```

**Step 2: Types Only Extractor** (4 hours)
```rust
// src/indexer/parser/types_only.rs
pub fn extract_types_only(code: &str, language: SupportedLanguage) -> Result<String> {
    let mut parser = Parser::new();
    parser.set_language(&language.tree_sitter_language())?;
    
    let tree = parser.parse(code, None).ok_or(ParserError::ParseError)?;
    let root = tree.root_node();
    
    let mut type_definitions = Vec::new();
    
    match language {
        SupportedLanguage::Rust => {
            collect_rust_types(root, code, &mut type_definitions);
        }
        SupportedLanguage::TypeScript => {
            collect_ts_types(root, code, &mut type_definitions);
        }
        // ... other languages
        _ => {}
    }
    
    Ok(type_definitions.join("\n\n"))
}

fn collect_rust_types(node: Node, code: &str, types: &mut Vec<String>) {
    match node.kind() {
        "struct_item" | "enum_item" | "type_item" | "trait_item" => {
            types.push(code[node.start_byte()..node.end_byte()].to_string());
        }
        _ => {
            for child in node.children(&mut node.walk()) {
                collect_rust_types(child, code, types);
            }
        }
    }
}

// MCP tool
async fn tool_read_types_only(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let file_path = args.get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("file required"))?;
    
    let full_path = ctx.root_path.join(file_path);
    let content = tokio::fs::read_to_string(&full_path).await?;
    let language = detect_language_from_path(&full_path)?;
    
    let types = extract_types_only(&content, language)?;
    
    Ok(json!({
        "file": file_path,
        "types": types,
        "original_lines": content.lines().count(),
        "types_lines": types.lines().count()
    }))
}
```

---

### Phase 0.5: Performance Optimization (3 days)
**Goal:** Server-side caching and batch operations

#### Features
8. **008_server_side_cache** (1.5 days)
   - LRU cache for read_file, search, symbols
   - TTL-based invalidation
   - 30-40% hit rate expected
   
13. **013_batch_operations** (1 day)
   - Batch read_file, search, get_symbols
   - 3-5× latency reduction
   
14. **014_query_optimization** (0.5 day)
   - Add missing indexes
   - Optimize hot queries

#### Implementation Steps

**Step 1: LRU Cache Implementation** (8 hours)
```rust
// src/daemon/cache.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct CacheEntry<T> {
    value: T,
    inserted_at: Instant,
    access_count: u32,
}

pub struct LruCache<T> {
    entries: HashMap<String, CacheEntry<T>>,
    max_size: usize,
    ttl: Duration,
}

impl<T: Clone> LruCache<T> {
    pub fn new(max_size: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            max_size,
            ttl,
        }
    }
    
    pub fn get(&mut self, key: &str) -> Option<T> {
        if let Some(entry) = self.entries.get_mut(key) {
            if entry.inserted_at.elapsed() < self.ttl {
                entry.access_count += 1;
                return Some(entry.value.clone());
            } else {
                self.entries.remove(key);
            }
        }
        None
    }
    
    pub fn put(&mut self, key: String, value: T) {
        if self.entries.len() >= self.max_size {
            self.evict_lru();
        }
        
        self.entries.insert(key, CacheEntry {
            value,
            inserted_at: Instant::now(),
            access_count: 0,
        });
    }
    
    pub fn invalidate(&mut self, pattern: &str) {
        self.entries.retain(|k, _| !k.contains(pattern));
    }
    
    fn evict_lru(&mut self) {
        if let Some((key_to_remove, _)) = self.entries.iter()
            .min_by_key(|(_, entry)| entry.access_count) {
            let key = key_to_remove.clone();
            self.entries.remove(&key);
        }
    }
}

pub struct CacheManager {
    read_file_cache: Arc<RwLock<LruCache<String>>>,
    search_cache: Arc<RwLock<LruCache<Value>>>,
    symbols_cache: Arc<RwLock<LruCache<Value>>>,
}

impl CacheManager {
    pub fn new() -> Self {
        Self {
            read_file_cache: Arc::new(RwLock::new(
                LruCache::new(100, Duration::from_secs(300))
            )),
            search_cache: Arc::new(RwLock::new(
                LruCache::new(50, Duration::from_secs(60))
            )),
            symbols_cache: Arc::new(RwLock::new(
                LruCache::new(50, Duration::from_secs(300))
            )),
        }
    }
    
    pub async fn get_read_file(&self, path: &str) -> Option<String> {
        self.read_file_cache.write().await.get(path)
    }
    
    pub async fn put_read_file(&self, path: String, content: String) {
        self.read_file_cache.write().await.put(path, content);
    }
    
    pub async fn invalidate_file(&self, path: &str) {
        self.read_file_cache.write().await.invalidate(path);
        self.symbols_cache.write().await.invalidate(path);
    }
}

// Integrate into tools
async fn tool_read_file(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let file_path = args.get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("file required"))?;
    
    // Check cache first
    if let Some(cached) = ctx.cache.get_read_file(file_path).await {
        return Ok(json!({ "file": file_path, "content": cached, "cached": true }));
    }
    
    // Cache miss, read from disk
    let full_path = ctx.root_path.join(file_path);
    let content = tokio::fs::read_to_string(&full_path).await?;
    
    // Store in cache
    ctx.cache.put_read_file(file_path.to_string(), content.clone()).await;
    
    Ok(json!({ "file": file_path, "content": content, "cached": false }))
}
```

**Step 2: Batch Operations** (6 hours)
```rust
// src/daemon/tools.rs
"batch_read_files" => tool_batch_read_files(args, ctx).await,

async fn tool_batch_read_files(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let files = args.get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("files array required"))?;
    
    let mut results = Vec::new();
    
    for file_val in files {
        if let Some(file_path) = file_val.as_str() {
            match tool_read_file(json!({ "file": file_path }), ctx).await {
                Ok(result) => results.push(result),
                Err(e) => results.push(json!({
                    "file": file_path,
                    "error": e.to_string()
                }))
            }
        }
    }
    
    Ok(json!({ "results": results }))
}
```

**Step 3: Query Optimization** (4 hours)
```sql
-- migrations/014_query_optimization.sql

-- Index for symbol lookups by name
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
CREATE INDEX IF NOT EXISTS idx_symbols_file_kind ON symbols(file_id, kind);

-- Index for reference lookups
CREATE INDEX IF NOT EXISTS idx_references_symbol ON references(symbol_id);
CREATE INDEX IF NOT EXISTS idx_references_file ON references(file_id);

-- Composite indexes for common queries
CREATE INDEX IF NOT EXISTS idx_symbols_name_kind ON symbols(name, kind);
CREATE INDEX IF NOT EXISTS idx_files_path_language ON files(path, language);

-- Analyze tables for query planner
ANALYZE;
```

---

### Phase 0.6: Incremental Indexing Enhancement (4 days)
**Goal:** 50-100× faster reindexing

#### Features
12. **012_incremental_indexing** (4 days)
   - Content hash tracking
   - Differential updates
   - Transactional operations

#### Implementation Steps

See detailed implementation in `012_incremental_indexing.md` documentation.

**Key Changes:**
1. Add `content_hash`, `last_indexed_at` to files table
2. Implement change detection based on hash comparison
3. Queue-based incremental updates
4. Transaction support for atomic updates
5. Cache invalidation on file changes

---

## 📋 Tool Schema Additions

Update `core_tools_list()` in `src/daemon/tools.rs`:

```rust
pub fn core_tools_list() -> Vec<Value> {
    vec![
        // ... existing tools ...
        
        json!({
            "name": "get_index_status",
            "description": "Get current index status, completeness, and metadata",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "validate_index",
            "description": "Validate index integrity and find issues",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "force_reindex",
            "description": "Force reindex of file, directory, or entire project",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "scope": { "type": "string", "enum": ["file", "directory", "project"] }
                }
            }
        }),
        json!({
            "name": "file_exists",
            "description": "Check if file exists in index (lightweight)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "symbol_exists",
            "description": "Check if symbol exists (lightweight)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string" },
                    "file": { "type": "string" }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "has_tests_for",
            "description": "Check if tests exist for a file",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string" }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "read_function_context",
            "description": "Read single function with its dependencies",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string" },
                    "function": { "type": "string" }
                },
                "required": ["file", "function"]
            }
        }),
        json!({
            "name": "read_types_only",
            "description": "Read only type definitions from a file",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string" }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "batch_read_files",
            "description": "Read multiple files in a single request",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["files"]
            }
        }),
    ]
}
```

---

## 🧪 Testing Strategy

### Unit Tests
```rust
// tests/phase0_tests.rs

#[tokio::test]
async fn test_index_status() {
    let ctx = setup_test_context().await;
    let result = tool_get_index_status(&ctx).await.unwrap();
    
    assert!(result.get("files").is_some());
    assert!(result.get("completeness_percent").is_some());
}

#[tokio::test]
async fn test_skeleton_generation() {
    let rust_code = r#"
        pub fn hello() {
            println!("Hello");
        }
    "#;
    
    let skeleton = generate_skeleton(rust_code, SupportedLanguage::Rust).unwrap();
    assert!(skeleton.contains("pub fn hello()"));
    assert!(skeleton.contains("{ /* ... */ }"));
    assert!(!skeleton.contains("println"));
}

#[tokio::test]
async fn test_cache_hit() {
    let cache = CacheManager::new();
    
    cache.put_read_file("test.rs".to_string(), "content".to_string()).await;
    
    let cached = cache.get_read_file("test.rs").await;
    assert_eq!(cached, Some("content".to_string()));
}
```

### Integration Tests
```bash
# Test full workflow
cargo test --test phase0_integration

# Performance benchmarks
cargo bench --bench phase0_perf
```

---

## 📈 Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Token savings (skeleton) | 3-5× | Compare token counts |
| Token savings (lightweight checks) | 95% | vs full read |
| Cache hit rate | 30-40% | Track hits/misses |
| Incremental indexing speedup | 50-100× | Time comparison |
| Index completeness visibility | 100% | Tool availability |

---

## 🚀 Deployment Plan

### Week 1: Index Quality + Token Efficiency Quick Wins
- Days 1-2: 001-003 (Index Status, Validate, Force Reindex)
- Days 3-4: 004-006 (Skeleton, Lightweight Checks, Search Scores)
- Day 5: Testing and bug fixes

### Week 2: Performance Infrastructure + Advanced Token Efficiency
- Days 1-2: 015-016 (Connection Pooling, Circuit Breaker)
- Days 3-4: 009-010 (Function Context, Types Only)
- Day 5: Testing and documentation

### Week 3: Performance Optimization
- Days 1-2: 008 (LRU Cache)
- Day 3: 013-014 (Batch Operations, Query Optimization)
- Days 4-5: Testing and optimization

### Week 4: Incremental Indexing
- Days 1-4: 012 (Incremental Indexing implementation)
- Day 5: Integration testing and polish

---

## ✅ Acceptance Criteria

- [ ] All 16 Phase 0 features implemented
- [ ] All unit tests passing
- [ ] Integration tests passing
- [ ] 50-70% token savings demonstrated
- [ ] Cache hit rate > 30%
- [ ] Incremental indexing 50× faster
- [ ] Documentation updated
- [ ] Performance benchmarks met

---

## 🔗 Related Documents

- [INDEX.md](docs/desc/INDEX.md) - Complete feature index
- [OVERVIEW.md](docs/desc/OVERVIEW.md) - Technical overview
- [Phase 0 Feature Specs](docs/desc/phase-0/) - Detailed specs for each feature

---

**Status:** Ready to implement  
**Next Step:** Begin Phase 0.1 - Index Quality & Visibility  
**Assigned To:** Development Team
