# Feature: server_side_cache - Кеширование для производительности

**ID:** PHASE0-008  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 0 (Quick Wins)

---

## 📋 Описание

Server-side LRU cache для часто запрашиваемых данных (read_file, get_symbols, search). Экономит 30-40% времени на повторных запросах, снижает нагрузку на SQLite/LanceDB.

### Проблема

**Текущее поведение:**
```
Request 1: read_file("src/main.rs") → SQLite query → 150ms
Request 2: read_file("src/main.rs") → SQLite query → 150ms (same file!)
Request 3: read_file("src/main.rs") → SQLite query → 150ms (again!)

Total: 450ms for same data
```

AI часто читает один и тот же файл несколько раз в одной сессии:
1. Skeleton read для обзора
2. Full read для деталей
3. Re-read после анализа других файлов

**Проблемы:**
- Повторные SQLite/disk reads
- Latency накапливается (3×150ms = 450ms)
- Ненужная нагрузка на storage
- Embedding model вызывается повторно

### Решение

**With cache:**
```
Request 1: read_file("src/main.rs") → Cache MISS → SQLite → 150ms → Cache store
Request 2: read_file("src/main.rs") → Cache HIT → 2ms ⚡
Request 3: read_file("src/main.rs") → Cache HIT → 2ms ⚡

Total: 154ms (66% faster!)
Cache hit rate: 67%
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Cache frequently accessed data (files, symbols, search results)
- ✅ LRU eviction (keep hot data, evict cold)
- ✅ TTL (time-to-live) per cache type
- ✅ Automatic invalidation on file changes
- ✅ Memory limit enforcement
- ✅ 30-40% latency reduction on repeated queries

### Non-Goals
- ❌ Not distributed cache (local to daemon process)
- ❌ Not persistent (in-memory only, cleared on restart)
- ❌ Not user-configurable cache size (fixed reasonable limits)
- ❌ Not caching large embeddings (too big for memory)

---

## 🏗️ Архитектура

### Cache Layers

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│  (read_file, get_symbols, search, ...)  │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Cache Manager  │
        │   (entry point) │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬───────────┐
     │           │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐ ┌──▼───┐
│  File    │ │Symbol│ │  Search   │ │Chunk │
│  Cache   │ │Cache │ │   Cache   │ │Cache │
└──────────┘ └──────┘ └───────────┘ └──────┘
     │           │           │           │
     └───────────┴───────────┴───────────┘
                     │
            ┌────────▼────────┐
            │  File Watcher   │
            │  (invalidation) │
            └─────────────────┘
```

### Cache Key Design

```rust
// Hierarchical key structure for efficient invalidation

FileCache key:    "file:{path}:{mtime}"
SymbolCache key:  "symbols:{file_id}"
SearchCache key:  "search:{query_hash}:{limit}"
ChunkCache key:   "chunk:{chunk_id}"

// Benefits:
// - File change → invalidate all related keys
// - Query params in key → different limits = different cache entries
// - mtime in key → automatic invalidation on file modification
```

---

## 📊 Data Model

### Cache Entry

```rust
#[derive(Clone)]
pub struct CacheEntry<T> {
    pub value: T,
    pub inserted_at: Instant,
    pub accessed_at: Instant,
    pub access_count: u64,
    pub size_bytes: usize,
}

impl<T> CacheEntry<T> {
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed() > ttl
    }
    
    pub fn touch(&mut self) {
        self.accessed_at = Instant::now();
        self.access_count += 1;
    }
}
```

### Cache Configuration

```rust
#[derive(Clone)]
pub struct CacheConfig {
    // File cache
    pub file_cache_size: usize,       // 100 MB
    pub file_cache_ttl: Duration,     // 5 minutes
    
    // Symbol cache
    pub symbol_cache_size: usize,     // 50 MB
    pub symbol_cache_ttl: Duration,   // 10 minutes
    
    // Search cache
    pub search_cache_size: usize,     // 20 MB
    pub search_cache_ttl: Duration,   // 2 minutes
    
    // Chunk cache
    pub chunk_cache_size: usize,      // 30 MB
    pub chunk_cache_ttl: Duration,    // 5 minutes
    
    // Global limits
    pub max_total_size: usize,        // 200 MB
    pub eviction_batch_size: usize,   // 10 entries
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            file_cache_size: 100 * 1024 * 1024,      // 100 MB
            file_cache_ttl: Duration::from_secs(300), // 5 min
            
            symbol_cache_size: 50 * 1024 * 1024,      // 50 MB
            symbol_cache_ttl: Duration::from_secs(600), // 10 min
            
            search_cache_size: 20 * 1024 * 1024,      // 20 MB
            search_cache_ttl: Duration::from_secs(120), // 2 min
            
            chunk_cache_size: 30 * 1024 * 1024,       // 30 MB
            chunk_cache_ttl: Duration::from_secs(300), // 5 min
            
            max_total_size: 200 * 1024 * 1024,        // 200 MB
            eviction_batch_size: 10,
        }
    }
}
```

---

## 🔧 API Specification

### Cache Manager API (Internal)

```rust
pub struct CacheManager {
    file_cache: Arc<RwLock<LruCache<String, CacheEntry<FileContent>>>>,
    symbol_cache: Arc<RwLock<LruCache<String, CacheEntry<Vec<Symbol>>>>>,
    search_cache: Arc<RwLock<LruCache<String, CacheEntry<SearchResponse>>>>,
    chunk_cache: Arc<RwLock<LruCache<String, CacheEntry<Chunk>>>>,
    
    config: CacheConfig,
    stats: Arc<RwLock<CacheStats>>,
}

impl CacheManager {
    // File operations
    pub async fn get_file(&self, path: &str, mtime: SystemTime) -> Option<FileContent>;
    pub async fn put_file(&self, path: &str, mtime: SystemTime, content: FileContent);
    pub async fn invalidate_file(&self, path: &str);
    
    // Symbol operations
    pub async fn get_symbols(&self, file_id: i64) -> Option<Vec<Symbol>>;
    pub async fn put_symbols(&self, file_id: i64, symbols: Vec<Symbol>);
    pub async fn invalidate_symbols(&self, file_id: i64);
    
    // Search operations
    pub async fn get_search(&self, query: &str, limit: usize) -> Option<SearchResponse>;
    pub async fn put_search(&self, query: &str, limit: usize, response: SearchResponse);
    pub async fn invalidate_all_searches(&self);
    
    // Stats
    pub async fn get_stats(&self) -> CacheStats;
    pub async fn clear_all(&self);
}
```

### Cache Statistics

```rust
#[derive(Clone, Serialize)]
pub struct CacheStats {
    // Overall stats
    pub total_size_bytes: usize,
    pub total_entries: usize,
    
    // Hit/miss rates
    pub file_hits: u64,
    pub file_misses: u64,
    pub file_hit_rate: f32,
    
    pub symbol_hits: u64,
    pub symbol_misses: u64,
    pub symbol_hit_rate: f32,
    
    pub search_hits: u64,
    pub search_misses: u64,
    pub search_hit_rate: f32,
    
    // Per-cache stats
    pub file_cache_size: usize,
    pub file_cache_entries: usize,
    
    pub symbol_cache_size: usize,
    pub symbol_cache_entries: usize,
    
    pub search_cache_size: usize,
    pub search_cache_entries: usize,
    
    // Evictions
    pub total_evictions: u64,
    pub ttl_evictions: u64,
    pub size_evictions: u64,
}

impl CacheStats {
    pub fn calculate_hit_rates(&mut self) {
        self.file_hit_rate = self.calculate_rate(self.file_hits, self.file_misses);
        self.symbol_hit_rate = self.calculate_rate(self.symbol_hits, self.symbol_misses);
        self.search_hit_rate = self.calculate_rate(self.search_hits, self.search_misses);
    }
    
    fn calculate_rate(&self, hits: u64, misses: u64) -> f32 {
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f32 / total as f32
        }
    }
}
```

### MCP Tool: get_cache_stats

Optional tool for monitoring:

```json
{
  "name": "get_cache_stats",
  "description": "Get cache statistics (hit rates, sizes, evictions)",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}
```

**Response:**
```json
{
  "total_size_bytes": 45678900,
  "total_entries": 234,
  "file_hit_rate": 0.67,
  "symbol_hit_rate": 0.82,
  "search_hit_rate": 0.45,
  "file_cache_entries": 120,
  "symbol_cache_entries": 89,
  "search_cache_entries": 25
}
```

---

## 💻 Implementation Details

### 1. LRU Cache Implementation

```rust
// src/cache/lru.rs

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

pub struct LruCache<K: Hash + Eq + Clone, V> {
    capacity: usize,
    cache: HashMap<K, CacheEntry<V>>,
    order: VecDeque<K>,
    current_size: usize,
}

impl<K: Hash + Eq + Clone, V> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            cache: HashMap::new(),
            order: VecDeque::new(),
            current_size: 0,
        }
    }
    
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if let Some(entry) = self.cache.get_mut(key) {
            // Touch entry (update access time)
            entry.touch();
            
            // Move to front (most recently used)
            self.move_to_front(key);
            
            Some(&entry.value)
        } else {
            None
        }
    }
    
    pub fn put(&mut self, key: K, value: V, size_bytes: usize) {
        // Remove old entry if exists
        if let Some(old_entry) = self.cache.remove(&key) {
            self.current_size -= old_entry.size_bytes;
            self.order.retain(|k| k != &key);
        }
        
        // Evict if necessary
        while self.current_size + size_bytes > self.capacity && !self.cache.is_empty() {
            self.evict_lru();
        }
        
        // Insert new entry
        let entry = CacheEntry {
            value,
            inserted_at: Instant::now(),
            accessed_at: Instant::now(),
            access_count: 0,
            size_bytes,
        };
        
        self.cache.insert(key.clone(), entry);
        self.order.push_front(key);
        self.current_size += size_bytes;
    }
    
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.cache.remove(key) {
            self.current_size -= entry.size_bytes;
            self.order.retain(|k| k != key);
            Some(entry.value)
        } else {
            None
        }
    }
    
    pub fn clear(&mut self) {
        self.cache.clear();
        self.order.clear();
        self.current_size = 0;
    }
    
    pub fn len(&self) -> usize {
        self.cache.len()
    }
    
    pub fn size(&self) -> usize {
        self.current_size
    }
    
    fn move_to_front(&mut self, key: &K) {
        // Remove from current position
        self.order.retain(|k| k != key);
        // Add to front
        self.order.push_front(key.clone());
    }
    
    fn evict_lru(&mut self) {
        if let Some(key) = self.order.pop_back() {
            if let Some(entry) = self.cache.remove(&key) {
                self.current_size -= entry.size_bytes;
            }
        }
    }
    
    pub fn evict_expired(&mut self, ttl: Duration) -> usize {
        let mut evicted = 0;
        let now = Instant::now();
        
        // Find expired keys
        let expired_keys: Vec<K> = self.cache.iter()
            .filter(|(_, entry)| now.duration_since(entry.inserted_at) > ttl)
            .map(|(k, _)| k.clone())
            .collect();
        
        // Remove expired entries
        for key in expired_keys {
            self.remove(&key);
            evicted += 1;
        }
        
        evicted
    }
}
```

### 2. Cache Manager

```rust
// src/cache/manager.rs

use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct CacheManager {
    file_cache: Arc<RwLock<LruCache<String, CacheEntry<FileContent>>>>,
    symbol_cache: Arc<RwLock<LruCache<String, CacheEntry<Vec<Symbol>>>>>,
    search_cache: Arc<RwLock<LruCache<String, CacheEntry<SearchResponse>>>>,
    chunk_cache: Arc<RwLock<LruCache<String, CacheEntry<Chunk>>>>,
    
    config: CacheConfig,
    stats: Arc<RwLock<CacheStats>>,
}

impl CacheManager {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            file_cache: Arc::new(RwLock::new(
                LruCache::new(config.file_cache_size)
            )),
            symbol_cache: Arc::new(RwLock::new(
                LruCache::new(config.symbol_cache_size)
            )),
            search_cache: Arc::new(RwLock::new(
                LruCache::new(config.search_cache_size)
            )),
            chunk_cache: Arc::new(RwLock::new(
                LruCache::new(config.chunk_cache_size)
            )),
            config,
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }
    
    /// Start background eviction task
    pub fn start_eviction_task(&self) {
        let file_cache = self.file_cache.clone();
        let symbol_cache = self.symbol_cache.clone();
        let search_cache = self.search_cache.clone();
        let chunk_cache = self.chunk_cache.clone();
        let config = self.config.clone();
        let stats = self.stats.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            
            loop {
                interval.tick().await;
                
                // Evict expired entries from all caches
                let mut total_evicted = 0;
                
                {
                    let mut cache = file_cache.write().await;
                    total_evicted += cache.evict_expired(config.file_cache_ttl);
                }
                
                {
                    let mut cache = symbol_cache.write().await;
                    total_evicted += cache.evict_expired(config.symbol_cache_ttl);
                }
                
                {
                    let mut cache = search_cache.write().await;
                    total_evicted += cache.evict_expired(config.search_cache_ttl);
                }
                
                {
                    let mut cache = chunk_cache.write().await;
                    total_evicted += cache.evict_expired(config.chunk_cache_ttl);
                }
                
                // Update stats
                if total_evicted > 0 {
                    let mut stats = stats.write().await;
                    stats.ttl_evictions += total_evicted as u64;
                    stats.total_evictions += total_evicted as u64;
                    
                    info!("Cache: evicted {} expired entries", total_evicted);
                }
            }
        });
    }
    
    // File cache operations
    
    pub async fn get_file(&self, path: &str, mtime: SystemTime) -> Option<FileContent> {
        let key = self.file_cache_key(path, mtime);
        
        let mut cache = self.file_cache.write().await;
        let result = cache.get(&key).map(|entry| entry.clone());
        
        // Update stats
        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.file_hits += 1;
        } else {
            stats.file_misses += 1;
        }
        
        result
    }
    
    pub async fn put_file(&self, path: &str, mtime: SystemTime, content: FileContent) {
        let key = self.file_cache_key(path, mtime);
        let size = self.estimate_size(&content);
        
        let mut cache = self.file_cache.write().await;
        cache.put(key, content, size);
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.file_cache_entries = cache.len();
        stats.file_cache_size = cache.size();
    }
    
    pub async fn invalidate_file(&self, path: &str) {
        // Invalidate all cache entries for this file
        // (different mtimes)
        
        let mut cache = self.file_cache.write().await;
        
        // Find all keys matching this path
        let keys_to_remove: Vec<String> = cache.iter()
            .filter(|(k, _)| k.starts_with(&format!("file:{}", path)))
            .map(|(k, _)| k.clone())
            .collect();
        
        for key in keys_to_remove {
            cache.remove(&key);
        }
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.file_cache_entries = cache.len();
        stats.file_cache_size = cache.size();
    }
    
    fn file_cache_key(&self, path: &str, mtime: SystemTime) -> String {
        let timestamp = mtime.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        format!("file:{}:{}", path, timestamp)
    }
    
    // Symbol cache operations
    
    pub async fn get_symbols(&self, file_id: i64) -> Option<Vec<Symbol>> {
        let key = format!("symbols:{}", file_id);
        
        let mut cache = self.symbol_cache.write().await;
        let result = cache.get(&key).map(|entry| entry.clone());
        
        // Update stats
        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.symbol_hits += 1;
        } else {
            stats.symbol_misses += 1;
        }
        
        result
    }
    
    pub async fn put_symbols(&self, file_id: i64, symbols: Vec<Symbol>) {
        let key = format!("symbols:{}", file_id);
        let size = self.estimate_size(&symbols);
        
        let mut cache = self.symbol_cache.write().await;
        cache.put(key, symbols, size);
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.symbol_cache_entries = cache.len();
        stats.symbol_cache_size = cache.size();
    }
    
    pub async fn invalidate_symbols(&self, file_id: i64) {
        let key = format!("symbols:{}", file_id);
        
        let mut cache = self.symbol_cache.write().await;
        cache.remove(&key);
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.symbol_cache_entries = cache.len();
        stats.symbol_cache_size = cache.size();
    }
    
    // Search cache operations
    
    pub async fn get_search(&self, query: &str, limit: usize) -> Option<SearchResponse> {
        let key = self.search_cache_key(query, limit);
        
        let mut cache = self.search_cache.write().await;
        let result = cache.get(&key).map(|entry| entry.clone());
        
        // Update stats
        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.search_hits += 1;
        } else {
            stats.search_misses += 1;
        }
        
        result
    }
    
    pub async fn put_search(&self, query: &str, limit: usize, response: SearchResponse) {
        let key = self.search_cache_key(query, limit);
        let size = self.estimate_size(&response);
        
        let mut cache = self.search_cache.write().await;
        cache.put(key, response, size);
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.search_cache_entries = cache.len();
        stats.search_cache_size = cache.size();
    }
    
    pub async fn invalidate_all_searches(&self) {
        let mut cache = self.search_cache.write().await;
        cache.clear();
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.search_cache_entries = 0;
        stats.search_cache_size = 0;
    }
    
    fn search_cache_key(&self, query: &str, limit: usize) -> String {
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        let hash = hasher.finish();
        
        format!("search:{}:{}", hash, limit)
    }
    
    // Stats
    
    pub async fn get_stats(&self) -> CacheStats {
        let mut stats = self.stats.read().await.clone();
        
        // Update sizes from caches
        stats.file_cache_size = self.file_cache.read().await.size();
        stats.file_cache_entries = self.file_cache.read().await.len();
        
        stats.symbol_cache_size = self.symbol_cache.read().await.size();
        stats.symbol_cache_entries = self.symbol_cache.read().await.len();
        
        stats.search_cache_size = self.search_cache.read().await.size();
        stats.search_cache_entries = self.search_cache.read().await.len();
        
        stats.total_size_bytes = stats.file_cache_size + 
                                 stats.symbol_cache_size + 
                                 stats.search_cache_size;
        
        stats.total_entries = stats.file_cache_entries + 
                              stats.symbol_cache_entries + 
                              stats.search_cache_entries;
        
        // Calculate hit rates
        stats.calculate_hit_rates();
        
        stats
    }
    
    pub async fn clear_all(&self) {
        self.file_cache.write().await.clear();
        self.symbol_cache.write().await.clear();
        self.search_cache.write().await.clear();
        self.chunk_cache.write().await.clear();
        
        let mut stats = self.stats.write().await;
        *stats = CacheStats::default();
    }
    
    // Size estimation
    
    fn estimate_size<T>(&self, value: &T) -> usize {
        // Approximate size in bytes
        // For MVP, use std::mem::size_of or fixed estimates
        
        std::mem::size_of_val(value)
    }
}
```

### 3. Integration with Storage Layer

```rust
// src/storage/sqlite.rs

impl SqliteStorage {
    // Existing method
    pub async fn read_file(&self, path: &str) -> Result<FileContent> {
        // NEW: Check cache first
        if let Some(cached) = self.cache_manager.get_file(path, mtime).await {
            return Ok(cached);
        }
        
        // Cache miss: query database
        let content = sqlx::query_as!(
            FileContent,
            "SELECT * FROM files WHERE path = ?",
            path
        )
        .fetch_one(&self.pool)
        .await?;
        
        // NEW: Store in cache
        self.cache_manager.put_file(path, mtime, content.clone()).await;
        
        Ok(content)
    }
    
    pub async fn get_symbols(&self, file_id: i64) -> Result<Vec<Symbol>> {
        // NEW: Check cache first
        if let Some(cached) = self.cache_manager.get_symbols(file_id).await {
            return Ok(cached);
        }
        
        // Cache miss: query database
        let symbols = sqlx::query_as!(
            Symbol,
            "SELECT * FROM symbols WHERE file_id = ?",
            file_id
        )
        .fetch_all(&self.pool)
        .await?;
        
        // NEW: Store in cache
        self.cache_manager.put_symbols(file_id, symbols.clone()).await;
        
        Ok(symbols)
    }
}
```

### 4. File Watcher Integration (Invalidation)

```rust
// src/watcher/mod.rs

impl FileWatcher {
    async fn handle_file_change(&self, path: &Path) {
        // Existing logic: reindex file
        // ...
        
        // NEW: Invalidate cache
        if let Some(path_str) = path.to_str() {
            self.cache_manager.invalidate_file(path_str).await;
            
            // Also invalidate symbols for this file
            if let Ok(file_id) = self.sqlite.get_file_id(path_str).await {
                self.cache_manager.invalidate_symbols(file_id).await;
            }
            
            // Invalidate all search results (they might include this file)
            self.cache_manager.invalidate_all_searches().await;
        }
    }
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_lru_cache_basic() {
    let mut cache = LruCache::new(100);
    
    cache.put("key1".to_string(), "value1", 10);
    cache.put("key2".to_string(), "value2", 10);
    
    assert_eq!(cache.get(&"key1".to_string()), Some(&"value1"));
    assert_eq!(cache.get(&"key2".to_string()), Some(&"value2"));
    assert_eq!(cache.len(), 2);
}

#[tokio::test]
async fn test_lru_cache_eviction() {
    let mut cache = LruCache::new(30);  // 30 bytes capacity
    
    cache.put("key1".to_string(), "value1", 10);
    cache.put("key2".to_string(), "value2", 10);
    cache.put("key3".to_string(), "value3", 10);
    
    assert_eq!(cache.len(), 3);
    
    // Add 4th item, should evict key1 (LRU)
    cache.put("key4".to_string(), "value4", 10);
    
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.get(&"key1".to_string()), None);  // Evicted
    assert_eq!(cache.get(&"key2".to_string()), Some(&"value2"));
}

#[tokio::test]
async fn test_lru_cache_ttl() {
    let mut cache = LruCache::new(100);
    
    cache.put("key1".to_string(), "value1", 10);
    
    // Wait for expiration
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let evicted = cache.evict_expired(Duration::from_millis(50));
    
    assert_eq!(evicted, 1);
    assert_eq!(cache.len(), 0);
}

#[tokio::test]
async fn test_cache_manager_file_cache() {
    let config = CacheConfig::default();
    let manager = CacheManager::new(config);
    
    let path = "test.rs";
    let mtime = SystemTime::now();
    let content = FileContent {
        path: path.to_string(),
        content: "fn main() {}".to_string(),
    };
    
    // Cache miss
    assert!(manager.get_file(path, mtime).await.is_none());
    
    // Put in cache
    manager.put_file(path, mtime, content.clone()).await;
    
    // Cache hit
    let cached = manager.get_file(path, mtime).await;
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().content, "fn main() {}");
    
    // Stats
    let stats = manager.get_stats().await;
    assert_eq!(stats.file_hits, 1);
    assert_eq!(stats.file_misses, 1);
    assert_eq!(stats.file_hit_rate, 0.5);
}

#[tokio::test]
async fn test_cache_invalidation() {
    let config = CacheConfig::default();
    let manager = CacheManager::new(config);
    
    let path = "test.rs";
    let mtime = SystemTime::now();
    let content = FileContent {
        path: path.to_string(),
        content: "fn main() {}".to_string(),
    };
    
    // Put in cache
    manager.put_file(path, mtime, content.clone()).await;
    
    // Verify cached
    assert!(manager.get_file(path, mtime).await.is_some());
    
    // Invalidate
    manager.invalidate_file(path).await;
    
    // Should be gone
    assert!(manager.get_file(path, mtime).await.is_none());
}

#[tokio::test]
async fn test_cache_stats() {
    let config = CacheConfig::default();
    let manager = CacheManager::new(config);
    
    // Populate cache
    for i in 0..10 {
        let path = format!("file{}.rs", i);
        let mtime = SystemTime::now();
        let content = FileContent {
            path: path.clone(),
            content: format!("content {}", i),
        };
        
        manager.put_file(&path, mtime, content).await;
    }
    
    // Access some files (hits)
    for i in 0..5 {
        let path = format!("file{}.rs", i);
        let mtime = SystemTime::now();
        manager.get_file(&path, mtime).await;
    }
    
    let stats = manager.get_stats().await;
    
    assert_eq!(stats.file_cache_entries, 10);
    assert!(stats.file_cache_size > 0);
    assert_eq!(stats.file_misses, 5);  // First access = miss
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_cache_integration_with_storage() {
    let (sqlite, lance) = setup_test_storage_with_cache().await;
    
    // First read: cache miss
    let start = Instant::now();
    let content1 = sqlite.read_file("test.rs").await.unwrap();
    let time1 = start.elapsed();
    
    // Second read: cache hit (should be faster)
    let start = Instant::now();
    let content2 = sqlite.read_file("test.rs").await.unwrap();
    let time2 = start.elapsed();
    
    assert_eq!(content1, content2);
    assert!(time2 < time1 / 2, "Cached read should be 2× faster");
    
    // Check stats
    let stats = sqlite.cache_manager.get_stats().await;
    assert_eq!(stats.file_hits, 1);
    assert_eq!(stats.file_misses, 1);
}

#[tokio::test]
async fn test_cache_invalidation_on_file_change() {
    let (sqlite, lance, watcher) = setup_test_environment_with_cache().await;
    
    // Read and cache
    sqlite.read_file("test.rs").await.unwrap();
    
    // Verify cached
    let stats = sqlite.cache_manager.get_stats().await;
    assert_eq!(stats.file_cache_entries, 1);
    
    // Modify file (triggers watcher)
    tokio::fs::write("test.rs", "// modified").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Cache should be invalidated
    let stats = sqlite.cache_manager.get_stats().await;
    assert_eq!(stats.file_cache_entries, 0);
}
```

### Performance Benchmarks

```rust
#[tokio::test]
async fn benchmark_cache_vs_no_cache() {
    let (sqlite_with_cache, _) = setup_test_storage_with_cache().await;
    let (sqlite_no_cache, _) = setup_test_storage_no_cache().await;
    
    // Warm up
    for i in 0..10 {
        let path = format!("file{}.rs", i);
        sqlite_with_cache.read_file(&path).await.unwrap();
        sqlite_no_cache.read_file(&path).await.unwrap();
    }
    
    // Benchmark: repeated reads
    let iterations = 100;
    
    // With cache
    let start = Instant::now();
    for _ in 0..iterations {
        for i in 0..10 {
            let path = format!("file{}.rs", i);
            sqlite_with_cache.read_file(&path).await.unwrap();
        }
    }
    let time_with_cache = start.elapsed();
    
    // Without cache
    let start = Instant::now();
    for _ in 0..iterations {
        for i in 0..10 {
            let path = format!("file{}.rs", i);
            sqlite_no_cache.read_file(&path).await.unwrap();
        }
    }
    let time_no_cache = start.elapsed();
    
    let speedup = time_no_cache.as_millis() as f64 / time_with_cache.as_millis() as f64;
    
    println!("With cache: {:?}", time_with_cache);
    println!("Without cache: {:?}", time_no_cache);
    println!("Speedup: {:.2}×", speedup);
    
    // Should be at least 2× faster with cache
    assert!(speedup > 2.0, "Cache speedup: {:.2}× (expected > 2×)", speedup);
}
```

---

## 📈 Success Metrics

### Performance
- ⏱️ 2× faster for repeated reads (cache hit)
- ⏱️ Cache hit latency: < 5ms
- ⏱️ Cache miss overhead: < 10ms

### Hit Rates (Target)
- 📊 File cache: 40-60% hit rate
- 📊 Symbol cache: 60-80% hit rate
- 📊 Search cache: 30-50% hit rate

### Memory
- 💾 Total cache size: < 200 MB
- 💾 No memory leaks (stable over time)
- 💾 LRU eviction working correctly

---

## 📚 Usage Examples

### For Users (via stats tool)

```typescript
// Check cache performance
const stats = await gofer.get_cache_stats();

console.log(`File cache hit rate: ${(stats.file_hit_rate * 100).toFixed(1)}%`);
console.log(`Symbol cache hit rate: ${(stats.symbol_hit_rate * 100).toFixed(1)}%`);
console.log(`Total cache size: ${(stats.total_size_bytes / 1024 / 1024).toFixed(1)} MB`);

if (stats.file_hit_rate < 0.3) {
  console.log("⚠️  Low cache hit rate - consider increasing cache size");
}
```

### For Developers (transparent)

Cache is transparent to tool users:

```typescript
// First call: cache miss (slower)
const file1 = await gofer.read_file({ file_path: "src/main.rs" });

// Second call: cache hit (faster)
const file2 = await gofer.read_file({ file_path: "src/main.rs" });

// Seamless experience!
```

---

## 🔄 Future Enhancements

### Phase 2: Smart Prefetching
- Prefetch related files when one is accessed
- Predict likely next reads based on patterns
- **Impact:** +20% hit rate

### Phase 3: Distributed Cache
- Share cache across multiple gofer instances
- Redis backend (optional)
- **Impact:** Multi-user scenarios

### Phase 4: Adaptive TTL
- Adjust TTL based on file change frequency
- Hot files: longer TTL
- Cold files: shorter TTL
- **Impact:** Better cache utilization

---

## ✅ Acceptance Criteria

- [ ] LRU cache implemented and tested
- [ ] Cache manager with file/symbol/search caches
- [ ] TTL-based expiration working
- [ ] Size-based eviction working
- [ ] File watcher invalidates cache on change
- [ ] Background eviction task runs
- [ ] Stats tracking hit/miss rates
- [ ] get_cache_stats MCP tool works
- [ ] Integration with read_file, get_symbols, search
- [ ] Performance: 2× faster on cache hits
- [ ] Memory: stays under 200 MB
- [ ] All tests pass
- [ ] Documentation complete

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Note:** Caching is critical for performance! 30-40% of queries are repeated within a session. This feature pays for itself immediately.
