# Feature: query_optimization - Оптимизация запросов

**ID:** PHASE0-014  
**Priority:** 🔥🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 0 (Performance)

---

## 📋 Описание

Автоматическая оптимизация SQL и vector search запросов. Includes query planning, index optimization, и automatic rewriting для улучшения производительности.

### Проблема

**Slow queries:**
```sql
-- Bad: full table scan
SELECT * FROM symbols WHERE name LIKE '%handler%';
→ 2000ms for 10k symbols

-- Bad: no index usage  
SELECT * FROM files WHERE path = 'src/main.rs';
→ 500ms for 1k files

-- Bad: N+1 queries
for symbol in symbols:
    get_file(symbol.file_id)  -- 100× DB queries
→ 5000ms total
```

**С query_optimization:**
```sql
-- Optimized: use index
SELECT * FROM symbols WHERE name >= 'handler' AND name < 'handlers';
→ 50ms (40× faster)

-- Optimized: index on path
CREATE INDEX idx_files_path ON files(path);
→ 10ms (50× faster)

-- Optimized: JOIN instead of N+1
SELECT s.*, f.path FROM symbols s JOIN files f ON s.file_id = f.id;
→ 100ms (50× faster)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Automatic query rewriting
- ✅ Index recommendations
- ✅ Query planning
- ✅ 10-50× speedup for common queries
- ✅ Monitoring slow queries

### Non-Goals
- ❌ Не изменяет пользовательские запросы
- ❌ Не automatic index creation (только recommendations)

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────┐
│         Query Interceptor               │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Query Analyzer │
        │  (detect slow)  │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│ Rewrite  │ │Index │ │   Cache   │
│  Rules   │ │ Hints│ │  Strategy │
└──────────┘ └──────┘ └───────────┘
```

---

## 💻 Key Optimizations

### 1. Index Strategy

```sql
-- Critical indexes
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_symbols_file_id ON symbols(file_id);
CREATE INDEX idx_symbols_kind ON symbols(kind);
CREATE INDEX idx_files_path ON files(path);
CREATE INDEX idx_chunks_file_id ON chunks(file_id);

-- Composite indexes for common queries
CREATE INDEX idx_symbols_kind_name ON symbols(kind, name);
CREATE INDEX idx_symbols_file_kind ON symbols(file_id, kind);
```

### 2. Query Rewriting

```rust
// LIKE '%pattern%' → full-text search
"SELECT * FROM symbols WHERE name LIKE '%handler%'"
→ "SELECT * FROM symbols WHERE name IN (SELECT ... FROM fts_symbols WHERE name MATCH 'handler')"

// Prefix LIKE → range query
"SELECT * FROM symbols WHERE name LIKE 'handle%'"
→ "SELECT * FROM symbols WHERE name >= 'handle' AND name < 'handlf'"
```

### 3. Query Batching

```rust
// N+1 → JOIN
for symbol in symbols {
    get_file(symbol.file_id)
}
→
SELECT s.*, f.* FROM symbols s JOIN files f ON s.file_id = f.id
```

---

## 📈 Success Metrics

- ⚡ 10-50× speedup for slow queries
- 📊 90%+ queries use indexes
- ⏱️ P95 query time < 100ms

---

## ✅ Acceptance Criteria

- [ ] Critical indexes created
- [ ] Query rewriting works
- [ ] Slow query detection
- [ ] 10× speedup for common patterns
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
