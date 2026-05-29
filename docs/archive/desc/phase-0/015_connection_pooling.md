# Feature: connection_pooling - Connection Pooling & Resource Management

**ID:** PHASE0-015  
**Priority:** 🔥🔥🔥 High  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 0 (Performance)

---

## 📋 Описание

Эффективное управление database connections, thread pools, и ресурсами для максимальной производительности под нагрузкой.

### Проблема

**Without connection pooling:**
```
Request 1: Open SQLite connection (50ms) + Query (10ms) + Close (5ms) = 65ms
Request 2: Open SQLite connection (50ms) + Query (10ms) + Close (5ms) = 65ms
Request 3: Open SQLite connection (50ms) + Query (10ms) + Close (5ms) = 65ms

Total: 195ms
Connection overhead: 165ms (85% waste!)
```

**С connection pooling:**
```
Request 1: Get from pool (1ms) + Query (10ms) + Return to pool (1ms) = 12ms
Request 2: Get from pool (1ms) + Query (10ms) + Return to pool (1ms) = 12ms
Request 3: Get from pool (1ms) + Query (10ms) + Return to pool (1ms) = 12ms

Total: 36ms (5.4× faster!)
Connection overhead: 6ms (17%)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ SQLite connection pool (10-20 connections)
- ✅ Thread pool for CPU-intensive tasks
- ✅ Async task executor optimization
- ✅ 5-10× faster под нагрузкой
- ✅ Resource limits (memory, connections)

### Non-Goals
- ❌ Не распределенный connection pool
- ❌ Не automatic scaling

---

## 💻 Implementation

### SQLite Connection Pool

```rust
// Use sqlx connection pool
let pool = SqlitePoolOptions::new()
    .max_connections(20)
    .min_connections(5)
    .acquire_timeout(Duration::from_secs(5))
    .idle_timeout(Duration::from_secs(300))
    .connect("sqlite://gofer.db")
    .await?;
```

### Thread Pool

```rust
// Rayon thread pool for CPU work
let thread_pool = rayon::ThreadPoolBuilder::new()
    .num_threads(num_cpus::get())
    .build()?;
```

### Resource Limits

```rust
pub struct ResourceLimits {
    max_memory_mb: usize,          // 2GB default
    max_concurrent_requests: usize, // 100 default
    max_embedding_batch_size: usize, // 32 default
}
```

---

## 📈 Success Metrics

- ⚡ 5-10× throughput improvement
- 📊 90%+ connection reuse rate
- 💾 Memory usage stays under limit

---

## ✅ Acceptance Criteria

- [ ] SQLite connection pool configured
- [ ] Thread pool for CPU tasks
- [ ] Resource limits enforced
- [ ] 5× throughput improvement
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
