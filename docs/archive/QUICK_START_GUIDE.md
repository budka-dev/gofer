# gofer MCP Phase 0 - Quick Start Guide

**Дата:** 2026-02-16  
**Статус:** Ready to Start  
**Цель:** Быстрый старт работы по Phase 0 Implementation

---

## 🎯 Что мы делаем?

Реализуем Phase 0: Foundation & Quick Wins для gofer MCP:
- ✅ **Index Quality** - видимость состояния индекса
- ✅ **Token Efficiency** - экономия 50-95% токенов
- ✅ **Performance** - кеширование, пулинг, оптимизация

### Ожидаемые результаты
- 50-70% экономия токенов
- 30-40% cache hit rate
- 50-100× быстрее инкрементальная индексация
- Полная прозрачность состояния индекса

---

## 📂 Структура проекта

```
gofer/
├── src/
│   ├── daemon/
│   │   ├── tools.rs          ← Главный файл для MCP tools
│   │   ├── cache.rs          ← [NEW] LRU cache manager
│   │   └── state.rs
│   ├── indexer/
│   │   ├── parser/
│   │   │   ├── skeleton.rs   ← ✅ Уже есть скелетизация
│   │   │   ├── function_context.rs  ← [NEW] Extraction отдельных функций
│   │   │   └── types_only.rs ← [NEW] Extraction только типов
│   │   ├── watcher.rs        ← Обновить для incremental indexing
│   │   ├── circuit_breaker.rs ← [NEW] Error recovery
│   │   └── service.rs
│   ├── storage/
│   │   ├── sqlite.rs         ← Оптимизировать pool settings
│   │   └── lance.rs
│   └── error.rs
├── migrations/
│   ├── 013_index_metadata.sql  ← [NEW] Metadata tracking
│   └── 014_query_optimization.sql ← [NEW] Indexes
├── docs/
│   ├── desc/
│   │   └── phase-0/          ← Детальные спецификации
│   ├── PHASE_0_IMPLEMENTATION_PLAN.md  ← Детальный план
│   └── QUICK_START_GUIDE.md  ← Этот документ
└── tests/
    └── phase0_tests.rs       ← [NEW] Тесты
```

---

## 🚀 С чего начать?

### Вариант 1: Быстрые победы (Recommended)
**Цель:** Получить результаты за 2-3 дня

1. **День 1: Index Quality Tools** (4 часа)
   ```bash
   # Создать миграцию
   touch migrations/013_index_metadata.sql
   
   # Добавить tools в src/daemon/tools.rs:
   # - get_index_status
   # - validate_index
   # - force_reindex
   ```

2. **День 2: Token Efficiency Quick Wins** (6 часов)
   ```bash
   # Skeleton tool уже есть, просто добавить wrapper
   # Добавить lightweight checks:
   # - file_exists
   # - symbol_exists
   # - has_tests_for
   
   # Улучшить search с confidence scores
   ```

3. **День 3: Тестирование** (4 часа)
   ```bash
   cargo test
   cargo run --bin gofer daemon
   # Проверить новые MCP tools через Python client
   ```

**Результат:** 3 новых инструмента для visibility + экономия токенов на lightweight checks

---

### Вариант 2: Performance First
**Цель:** Улучшить производительность в первую очередь

1. **День 1-2: Server-side Cache** (8 часов)
   ```bash
   # Создать src/daemon/cache.rs
   # Реализовать LRU cache
   # Интегрировать в tool_read_file, tool_search
   ```

2. **День 3: Circuit Breaker** (6 часов)
   ```bash
   # Создать src/indexer/circuit_breaker.rs
   # Добавить retry logic в embedder
   ```

3. **День 4: Query Optimization** (4 часа)
   ```bash
   # Создать migrations/014_query_optimization.sql
   # Добавить недостающие индексы
   ```

**Результат:** 30-40% cache hit rate, устойчивость к ошибкам embeddings

---

### Вариант 3: Full Phase 0 (4 недели)
**Цель:** Полная реализация всех 16 фич

См. детальный план в [PHASE_0_IMPLEMENTATION_PLAN.md](./PHASE_0_IMPLEMENTATION_PLAN.md)

---

## 📝 Checklist для каждой фичи

### Стандартный workflow:

1. **Прочитать спецификацию**
   ```bash
   cat docs/desc/phase-0/001_get_index_status.md
   ```

2. **Создать миграцию (если нужна)**
   ```bash
   touch migrations/013_<feature_name>.sql
   # Добавить SQL для новых таблиц/индексов
   ```

3. **Реализовать функционал**
   ```bash
   # Добавить в src/daemon/tools.rs:
   # 1. Функцию tool_<feature_name>
   # 2. Добавить в dispatch()
   # 3. Добавить в core_tools_list()
   ```

4. **Написать тесты**
   ```bash
   # В tests/phase0_tests.rs
   #[tokio::test]
   async fn test_<feature_name>() {
       // ...
   }
   ```

5. **Проверить**
   ```bash
   cargo test
   cargo build --release
   cargo run --bin gofer daemon
   
   # Python integration test
   python tests/mcp_integration_test.py
   ```

6. **Задокументировать**
   - Обновить README.md (если нужно)
   - Добавить примеры использования

---

## 🔧 Полезные команды

### Сборка и запуск
```bash
# Обычная сборка (медленнее, но быстрее работает)
cargo build --release

# Быстрая сборка для разработки
cargo build --profile release-dev

# Запуск daemon
cargo run --bin gofer daemon

# Проверка индексации
cargo run --bin gofer check
```

### Тестирование
```bash
# Все тесты
cargo test

# Только Phase 0 тесты
cargo test phase0

# Конкретный тест
cargo test test_index_status

# С логами
RUST_LOG=debug cargo test test_index_status -- --nocapture
```

### Database
```bash
# Применить миграции
sqlx migrate run

# Проверить схему
sqlite3 ~/.gofer/projects/<project_hash>/gofer.db ".schema"

# Посмотреть данные
sqlite3 ~/.gofer/projects/<project_hash>/gofer.db "SELECT * FROM index_metadata;"
```

---

## 🎯 Приоритеты для первых 3 дней

### День 1: Index Status (4 часа)
**Цель:** Понять, что проиндексировано

```rust
// src/daemon/tools.rs

// 1. Добавить в dispatch():
"get_index_status" => tool_get_index_status(ctx).await,

// 2. Реализовать:
async fn tool_get_index_status(ctx: &ToolContext<'_>) -> Result<Value> {
    let file_count = sqlx::query_scalar!("SELECT COUNT(*) FROM files")
        .fetch_one(&ctx.sqlite.pool).await?;
    
    let symbol_count = sqlx::query_scalar!("SELECT COUNT(*) FROM symbols")
        .fetch_one(&ctx.sqlite.pool).await?;
    
    Ok(json!({
        "files": file_count,
        "symbols": symbol_count,
        "status": "complete"
    }))
}

// 3. Добавить в core_tools_list():
json!({
    "name": "get_index_status",
    "description": "Get index status and completeness",
    "inputSchema": { "type": "object", "properties": {} }
})
```

**Тест:**
```bash
cargo test test_index_status
cargo run --bin gofer daemon
# Вызвать через MCP client
```

---

### День 2: Lightweight Checks (6 часов)
**Цель:** Быстрые проверки без полного чтения

```rust
// src/daemon/tools.rs

async fn tool_file_exists(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let path = args.get("path").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("path required"))?;
    
    let exists = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM files WHERE path = ?", path
    ).fetch_one(&ctx.sqlite.pool).await? > 0;
    
    Ok(json!({ "exists": exists }))
}

async fn tool_symbol_exists(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("symbol required"))?;
    
    let exists = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM symbols WHERE name = ?", symbol
    ).fetch_one(&ctx.sqlite.pool).await? > 0;
    
    Ok(json!({ "exists": exists }))
}
```

**Экономия:** 95% токенов vs полное чтение файла

---

### День 3: Cache Implementation (8 часов)
**Цель:** 30-40% снижение latency

```rust
// src/daemon/cache.rs (новый файл)

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct LruCache<T> {
    entries: HashMap<String, (T, Instant)>,
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
        if let Some((value, inserted_at)) = self.entries.get(key) {
            if inserted_at.elapsed() < self.ttl {
                return Some(value.clone());
            }
        }
        None
    }
    
    pub fn put(&mut self, key: String, value: T) {
        if self.entries.len() >= self.max_size {
            // Evict oldest
            if let Some(oldest_key) = self.entries.keys().next().cloned() {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(key, (value, Instant::now()));
    }
}
```

---

## 📊 Как измерить успех?

### Token Savings
```python
# Сравнить размеры ответов
full_content = call_tool("read_file", {"file": "src/main.rs"})
skeleton = call_tool("skeleton", {"file": "src/main.rs"})

savings = (len(full_content) - len(skeleton)) / len(full_content) * 100
print(f"Token savings: {savings:.1f}%")  # Ожидаем 60-80%
```

### Cache Hit Rate
```bash
# В логах искать
grep "cache hit" ~/.gofer/daemon.log | wc -l
grep "cache miss" ~/.gofer/daemon.log | wc -l

# Ожидаем hit rate > 30%
```

### Incremental Indexing Speed
```bash
# Засечь время
time cargo run --bin gofer index
# Full: ~2-3 минуты

# Изменить 1 файл
echo "// test" >> src/test.rs

# Incremental должен быть < 5 секунд
```

---

## 🆘 Troubleshooting

### Problem: Миграции не применяются
```bash
# Проверить версию
sqlx migrate info

# Применить вручную
sqlx migrate run

# Или удалить базу и пересоздать
rm -rf ~/.gofer/projects/<hash>/
cargo run --bin gofer index
```

### Problem: Тесты падают
```bash
# Чистая пересборка
cargo clean
cargo build

# Проверить зависимости
cargo check

# Запустить с подробными логами
RUST_LOG=debug cargo test -- --nocapture
```

### Problem: MCP tools не видны
```bash
# Проверить, что добавлены в core_tools_list()
cargo run --bin gofer daemon --verbose

# Смотреть логи
tail -f ~/.gofer/daemon.log
```

---

## 📚 Полезные ссылки

- [PHASE_0_IMPLEMENTATION_PLAN.md](./PHASE_0_IMPLEMENTATION_PLAN.md) - Детальный план
- [INDEX.md](./desc/INDEX.md) - Все фичи
- [OVERVIEW.md](./desc/OVERVIEW.md) - Техническая архитектура
- [Phase 0 Specs](./desc/phase-0/) - Спецификации каждой фичи

---

## ✅ Критерии готовности Phase 0

- [ ] 001-003: Index Quality tools работают
- [ ] 004-006: Token Efficiency tools (skeleton, lightweight checks) работают
- [ ] 008: Server-side cache внедрен, hit rate > 30%
- [ ] 009-010: Function context и types-only extraction работают
- [ ] 012: Incremental indexing 50× быстрее
- [ ] 013-014: Batch operations и query optimization
- [ ] 015-016: Connection pooling и circuit breaker
- [ ] Все тесты проходят
- [ ] Документация обновлена

---

## 🚀 Начинаем!

Рекомендуемый порядок:

1. **Прочитать этот guide** ✅
2. **Выбрать подход:**
   - Quick wins (2-3 дня)
   - Performance first (4-5 дней)
   - Full Phase 0 (4 недели)
3. **Начать с Day 1:**
   ```bash
   cd /home/gofer/vibe/gofer
   git checkout -b feature/phase-0-index-quality
   
   # Создать миграцию
   touch migrations/013_index_metadata.sql
   
   # Начать реализацию
   code src/daemon/tools.rs
   ```

---

**Good luck!** 🚀

Если возникнут вопросы, смотрите детальные спецификации в `docs/desc/phase-0/`
