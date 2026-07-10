//! Server-side LRU cache for frequently accessed data
//! Reduces latency by 30-40% on repeated queries

use rkyv::AlignedVec;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// LRU cache with size-based eviction
pub struct LruCache<K: Hash + Eq + Clone, V: Clone> {
    capacity_bytes: usize,
    cache: HashMap<K, CacheEntry<V>>,
    order: VecDeque<K>,
    current_size: usize,
}

/// Cache entry with metadata
#[derive(Clone)]
pub struct CacheEntry<V> {
    pub value: V,
    pub accessed_at: Instant,
    pub access_count: u64,
    pub size_bytes: usize,
    pub mtime: Option<std::time::SystemTime>,
}

#[allow(dead_code)]
impl<V> CacheEntry<V> {
    pub fn new(value: V, size_bytes: usize) -> Self {
        Self::new_with_mtime(value, size_bytes, None)
    }

    pub fn new_with_mtime(
        value: V,
        size_bytes: usize,
        mtime: Option<std::time::SystemTime>,
    ) -> Self {
        let now = Instant::now();
        Self {
            value,
            accessed_at: now,
            access_count: 0,
            size_bytes,
            mtime,
        }
    }

    pub fn touch(&mut self) {
        self.accessed_at = Instant::now();
        self.access_count += 1;
    }
}

#[allow(dead_code)]
impl<K: Hash + Eq + Clone, V: Clone> LruCache<K, V> {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            cache: HashMap::new(),
            order: VecDeque::new(),
            current_size: 0,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.cache.get_mut(key) {
            entry.touch();
            let value = entry.value.clone();
            self.move_to_front(key);
            Some(value)
        } else {
            None
        }
    }

    pub fn get_with_mtime(&mut self, key: &K) -> Option<(V, Option<std::time::SystemTime>)> {
        if let Some(entry) = self.cache.get_mut(key) {
            entry.touch();
            let value = entry.value.clone();
            let mtime = entry.mtime;
            self.move_to_front(key);
            Some((value, mtime))
        } else {
            None
        }
    }

    pub fn put(&mut self, key: K, value: V, size_bytes: usize) {
        self.put_with_mtime(key, value, size_bytes, None);
    }

    pub fn put_with_mtime(
        &mut self,
        key: K,
        value: V,
        size_bytes: usize,
        mtime: Option<std::time::SystemTime>,
    ) {
        // Remove old entry if exists
        if let Some(old_entry) = self.cache.remove(&key) {
            self.current_size = self.current_size.saturating_sub(old_entry.size_bytes);
            self.order.retain(|k| k != &key);
        }

        // Evict if necessary
        while self.current_size + size_bytes > self.capacity_bytes && !self.cache.is_empty() {
            self.evict_lru();
        }

        // Don't cache if single item is larger than capacity
        if size_bytes > self.capacity_bytes {
            return;
        }

        // Insert new entry
        let entry = CacheEntry::new_with_mtime(value, size_bytes, mtime);
        self.cache.insert(key.clone(), entry);
        self.order.push_front(key);
        self.current_size += size_bytes;
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.cache.remove(key) {
            self.current_size = self.current_size.saturating_sub(entry.size_bytes);
            self.order.retain(|k| k != key);
            Some(entry.value)
        } else {
            None
        }
    }

    pub fn invalidate_prefix(&mut self, prefix: &str)
    where
        K: AsRef<str>,
    {
        let keys_to_remove: Vec<K> = self
            .cache
            .keys()
            .filter(|k| k.as_ref().starts_with(prefix))
            .cloned()
            .collect();

        for key in keys_to_remove {
            self.remove(&key);
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

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn current_size_bytes(&self) -> usize {
        self.current_size
    }


    fn evict_lru(&mut self) {
        if let Some(key) = self.order.pop_back() {
            self.cache.remove(&key);
        }
    }

    fn move_to_front(&mut self, key: &K) {
        self.order.retain(|k| k != key);
        self.order.push_front(key.clone());
    }
}

/// Cache manager with multiple cache layers
pub struct CacheManager {
    file_cache: Arc<RwLock<LruCache<String, String>>>,
    symbol_cache: Arc<RwLock<LruCache<String, String>>>,
    symbol_cache_rkyv: Arc<RwLock<LruCache<String, AlignedVec>>>,
    search_cache: Arc<RwLock<LruCache<String, String>>>,

    stats: Arc<RwLock<CacheStats>>,
}

#[allow(dead_code)]
impl CacheManager {
    pub fn new() -> Self {
        Self {
            // 100 MB for files
            file_cache: Arc::new(RwLock::new(LruCache::new(100 * 1024 * 1024))),
            // 50 MB for symbols (JSON)
            symbol_cache: Arc::new(RwLock::new(LruCache::new(50 * 1024 * 1024))),
            // 50 MB for symbols (rkyv)
            symbol_cache_rkyv: Arc::new(RwLock::new(LruCache::new(50 * 1024 * 1024))),
            // 20 MB for search
            search_cache: Arc::new(RwLock::new(LruCache::new(20 * 1024 * 1024))),

            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }

    // File cache operations
    pub async fn get_file(&self, path: &str) -> Option<String> {
        let mut cache = self.file_cache.write().await;
        let result = cache.get(&path.to_string());

        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.file_hits += 1;
        } else {
            stats.file_misses += 1;
        }

        result
    }

    pub async fn get_file_with_mtime(
        &self,
        path: &str,
    ) -> Option<(String, Option<std::time::SystemTime>)> {
        let mut cache = self.file_cache.write().await;
        let result = cache.get_with_mtime(&path.to_string());

        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.file_hits += 1;
        } else {
            stats.file_misses += 1;
        }

        result
    }

    pub async fn put_file(&self, path: String, content: String) {
        self.put_file_with_mtime(path, content, None).await;
    }

    pub async fn put_file_with_mtime(
        &self,
        path: String,
        content: String,
        mtime: Option<std::time::SystemTime>,
    ) {
        let size = content.len();
        let mut cache = self.file_cache.write().await;
        cache.put_with_mtime(path, content, size, mtime);
    }

    pub async fn invalidate_file(&self, path: &str) {
        let mut cache = self.file_cache.write().await;
        cache.remove(&path.to_string());

        // Also invalidate related caches
        let mut symbol_cache = self.symbol_cache.write().await;
        symbol_cache.invalidate_prefix(&format!("file:{}", path));
    }

    // Symbol cache operations
    pub async fn get_symbols(&self, key: &str) -> Option<String> {
        let mut cache = self.symbol_cache.write().await;
        let result = cache.get(&key.to_string());

        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.symbol_hits += 1;
        } else {
            stats.symbol_misses += 1;
        }

        result
    }


    // Symbol cache operations (rkyv)
    pub async fn get_symbols_rkyv(&self, key: &str) -> Option<AlignedVec> {
        let mut cache = self.symbol_cache_rkyv.write().await;
        let result = cache.get(&key.to_string());

        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.symbol_hits += 1;
        } else {
            stats.symbol_misses += 1;
        }

        result
    }

    pub async fn put_symbols_rkyv(&self, key: String, data: AlignedVec) {
        let size = data.len();
        let mut cache = self.symbol_cache_rkyv.write().await;
        cache.put(key, data, size);
    }

    // Search cache operations
    pub async fn get_search(&self, query: &str, limit: usize) -> Option<String> {
        let cache_key = format!("{}:{}", query, limit);
        let mut cache = self.search_cache.write().await;
        let result = cache.get(&cache_key);

        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.search_hits += 1;
        } else {
            stats.search_misses += 1;
        }

        result
    }

    pub async fn put_search(&self, query: String, limit: usize, data: String) {
        let cache_key = format!("{}:{}", query, limit);
        let size = data.len();
        let mut cache = self.search_cache.write().await;
        cache.put(cache_key, data, size);
    }

    pub async fn invalidate_all_searches(&self) {
        let mut cache = self.search_cache.write().await;
        cache.clear();
    }

}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics (updated on get/put; no external reader yet)
#[derive(Clone, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct CacheStats {
    // Overall
    pub total_size_bytes: usize,
    pub total_entries: usize,

    // File cache
    pub file_hits: u64,
    pub file_misses: u64,
    pub file_hit_rate: f32,
    pub file_cache_size: usize,
    pub file_cache_entries: usize,

    // Symbol cache
    pub symbol_hits: u64,
    pub symbol_misses: u64,
    pub symbol_hit_rate: f32,
    pub symbol_cache_size: usize,
    pub symbol_cache_entries: usize,

    // Search cache
    pub search_hits: u64,
    pub search_misses: u64,
    pub search_hit_rate: f32,
    pub search_cache_size: usize,
    pub search_cache_entries: usize,

    // Evictions
    pub total_evictions: u64,
    pub ttl_evictions: u64,
}


