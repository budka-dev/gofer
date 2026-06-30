# Storage API

Публичный API двух движков хранилища. Полезно тем, кто пишет handler'ы или хочет дёрнуть индекс напрямую без MCP.

- `SqliteStorage` (`src/storage/sqlite.rs`, ~2300 строк) — метаданные, символы, граф ссылок, зависимости, диагностика, кеш.
- `LanceStorage` (`src/storage/lance.rs`, ~800 строк) — векторное хранилище для семантического поиска.

Обе структуры — `Arc`-shared, безопасны для параллельных запросов через `&self`. Внутри живут sqlx pool и lancedb connection.

Все типы возвращаемых структур — см. [models.md](models.md).

## SqliteStorage

### Инициализация и обслуживание

```rust
pub async fn new(db_path: &str) -> Result<Self>
pub async fn migrate(&self) -> Result<()>
pub async fn health_check(&self) -> Result<()>
pub async fn check_integrity(&self) -> Result<()>
```

`new` открывает (или создаёт) БД и автоматически выставляет PRAGMA-параметры. `migrate` применяет все embedded миграции (см. [architecture.md::Миграции SQLite](architecture.md#миграции-sqlite)). `health_check` — простой `SELECT 1`. `check_integrity` — `PRAGMA integrity_check`, тяжелее, гонять редко.

```rust
pub fn pool(&self) -> &SqlitePool
pub fn metrics(&self) -> &QueryMetrics
```

`pool` нужен для произвольных запросов через `sqlx::query!`. `metrics` отдаёт лок-фри счётчик latency.

```rust
pub async fn with_write_retry<F, Fut, T>(&self, mut op: F) -> Result<T>
```

Обёртка над пишущей операцией с retry на `SQLITE_BUSY`. Используй её для всего, что пишет, особенно когда watcher активен.

### Файлы

```rust
pub async fn upsert_file(...) -> Result<i64>           // вернёт file_id
pub async fn get_file(&self, path: &str) -> Result<Option<IndexedFile>>
pub async fn get_file_by_id(&self, id: i64) -> Result<Option<IndexedFile>>
pub async fn needs_reindex(&self, path: &str, content_hash: &str) -> Result<bool>
pub async fn get_all_file_hashes(&self) -> Result<HashMap<String, (String, i64)>>
pub async fn delete_file(&self, path: &str) -> Result<()>
pub async fn get_file_count(&self) -> Result<i64>
pub async fn update_file_domain(...) -> Result<()>
pub async fn reset_all_indexing_status(&self) -> Result<()>
```

`upsert_file` записывает метаданные и возвращает `file_id` — используй его для последующих `insert_symbols`/`insert_references`. `get_all_file_hashes` — массовый prefetch для pipeline.

### Символы

```rust
pub async fn insert_symbols(&self, file_id: i64, symbols: &[Symbol]) -> Result<()>
pub async fn search_symbols(&self, query: &str, limit: i32) -> Result<Vec<Symbol>>
pub async fn search_symbols_with_path(...) -> Result<Vec<SymbolWithPath>>
pub async fn search_symbols_with_path_filter(...) -> Result<Vec<SymbolWithPath>>
pub async fn get_symbols(...) -> Result<Vec<SymbolWithPath>>     // пагинированный
pub async fn get_file_symbols(&self, file_id: i64) -> Result<Vec<Symbol>>
pub async fn get_symbol_by_name(&self, name: &str) -> Result<Vec<Symbol>>
pub async fn get_symbol_by_id(&self, id: i64) -> Result<Option<Symbol>>
pub async fn find_symbol_by_name_and_file(...) -> Result<Option<Symbol>>
pub async fn find_symbol_at_line(&self, file_path: &str, line: i32) -> Result<Option<Symbol>>
```

`insert_symbols` пишет батчем — не вызывай в цикле по одному. `search_symbols` — точное/префиксное; `search_symbols_with_path` отдаёт готовый join с `file_path` (нужно для MCP). `find_symbol_at_line` — обратная навигация: где этот символ начинается.

### Граф ссылок

```rust
pub async fn insert_references(&self, symbol_id: i64, refs: &[SymbolReference]) -> Result<()>
pub async fn get_outgoing_references(&self, symbol_id: i64) -> Result<Vec<SymbolReference>>
pub async fn get_incoming_references(&self, symbol_name: &str) -> Result<Vec<SymbolReference>>
pub async fn get_references_by_name(...) -> Result<Vec<ReferenceWithPath>>
pub async fn resolve_references(&self) -> Result<u64>   // возвращает число разрешённых
```

`resolve_references` — пост-проход после полного sync'а: пытается заполнить `target_symbol_id` там, где он был `None`. Вызывается из pipeline автоматически.

### Зависимости

```rust
pub async fn upsert_dependency(&self, dep: &Dependency) -> Result<()>
pub async fn get_dependencies(&self) -> Result<Vec<Dependency>>
pub async fn get_dependencies_filtered(...) -> Result<Vec<Dependency>>
pub async fn get_dependencies_by_ecosystem(&self, ecosystem: &str) -> Result<Vec<Dependency>>
pub async fn get_unused_dependencies(&self) -> Result<Vec<Dependency>>

pub async fn record_dependency_usage(...) -> Result<()>
pub async fn clear_dependency_usage(&self, file_id: i64) -> Result<()>
pub async fn get_dependency_usages(&self, dep_name: &str) -> Result<Vec<DependencyUsage>>
pub async fn get_dependency_usage(&self, dep_name: &str) -> Result<Vec<DependencyUsageInfo>>
pub async fn get_dependency_impact(...) -> Result<...>
pub async fn get_file_dependencies(&self, file_id: i64) -> Result<Vec<(String, String, i32)>>
```

Зависимости разделены на две таблицы: сами dep'ы и их usage по файлам. `clear_dependency_usage` чистит перед перезаписью при инкрементальном sync'е.

### Knowledge: rules и golden samples

```rust
pub async fn upsert_rules(&self, rules: &[Rule], source: &str) -> Result<()>
pub async fn get_rules(&self) -> Result<Vec<Rule>>
pub async fn mark_golden_sample(...) -> Result<()>
pub async fn get_golden_samples(&self) -> Result<Vec<(String, Option<String>)>>
```

Используются для впрыска контекста в LLM через ресурс `project://context`.

### Диагностика (компилятор)

```rust
pub async fn clear_active_errors(&self) -> Result<()>
pub async fn clear_file_errors(&self, file_path: &str) -> Result<()>
pub async fn insert_error(...) -> Result<()>
pub async fn get_active_errors(&self) -> Result<Vec<ActiveError>>
pub async fn get_file_errors(&self, file_path: &str) -> Result<Vec<ActiveError>>
pub async fn count_errors(&self) -> Result<(i64, i64)>   // (errors, warnings)
pub async fn get_errors(...) -> Result<Vec<ActiveError>> // пагинированный
```

Кеш компиляторных диагностик. Таблица сохранена в схеме SQLite, но инструменты записи в неё не входят в текущий read-only MCP-API.

### Конфиг-ключи

```rust
pub async fn upsert_config_key(...) -> Result<()>
pub async fn get_config_keys(&self) -> Result<Vec<ConfigKey>>
```

Эвристический реестр ключей из `.env.example`, `config.rs` структур и подобного.

### Vue tree

```rust
pub async fn upsert_vue_tree(...) -> Result<()>
pub async fn get_vue_tree(&self, file_path: &str) -> Result<Option<VueTree>>
```

Иерархическая структура Vue-компонента, сохранённая при парсинге `.vue`.

### Domains и cross-stack

```rust
pub async fn insert_entity_link(...) -> Result<()>
pub async fn insert_api_endpoint(...) -> Result<()>
pub async fn insert_frontend_api_call(...) -> Result<()>
pub async fn get_api_endpoints(&self) -> Result<Vec<ApiEndpointInfo>>
pub async fn get_frontend_api_calls(&self) -> Result<Vec<FrontendApiCallInfo>>
pub async fn get_frontend_links(...) -> Result<...>
pub async fn get_backend_links(...) -> Result<...>
pub async fn get_domain_stats(&self) -> Result<Vec<(String, i64)>>
```

### Structural fingerprinting

```rust
pub async fn upsert_type_fingerprint(...) -> Result<()>
pub async fn get_fingerprints_by_language(...) -> Result<Vec<TypeFingerprint>>
pub async fn clear_file_fingerprints(&self, file_id: i64) -> Result<()>

pub async fn upsert_cross_stack_link(...) -> Result<()>
pub async fn get_cross_stack_links_for_file(...) -> Result<Vec<CrossStackLink>>
pub async fn get_cross_stack_links_by_type(...) -> Result<Vec<CrossStackLink>>
pub async fn clear_structural_links(&self) -> Result<()>
```

Связи backend ↔ frontend по Jaccard-сходству нормализованных полей. См. миграцию `007_structural_links.sql`.

### Subprojects (монорепо)

```rust
pub async fn upsert_subproject(...) -> Result<i64>
pub async fn set_file_subproject(&self, file_id: i64, subproject_id: i64) -> Result<()>
pub async fn list_subprojects(&self) -> Result<Vec<SubprojectRecord>>
pub async fn clear_subprojects(&self) -> Result<()>
```

Для workspace-members в Cargo, lerna-style monorepo. Заполняется при scan'е.

### Index metadata

```rust
pub async fn get_index_meta(&self, key: &str) -> Result<Option<String>>
pub async fn set_index_meta(&self, key: &str, value: &str) -> Result<()>
```

Произвольный KV для меты: версия индекса, версия эмбеддера, дата последнего полного sync'а.

### Chunk cache

```rust
pub async fn get_cached_embeddings(...) -> Result<HashMap<String, Vec<f32>>>
pub async fn store_cached_embeddings(&self, entries: &[(String, Vec<f32>)]) -> Result<()>
pub async fn clear_chunk_cache(&self) -> Result<()>
pub async fn get_chunk_cache_stats(&self) -> Result<(i64, i64)>   // (count, total_bytes)
pub async fn evict_chunk_cache_to_limit(&self, max_entries: i64) -> Result<i64>
pub async fn evict_chunk_cache_by_age(&self, max_age_days: i64) -> Result<i64>
```

`get_cached_embeddings` — по `content_hash`, чтобы пропустить переэмбеддинг unchanged-чанков. `evict_*` зовутся в конце pipeline, по умолчанию лимиты 100k записей и 30 дней.

### Audit log

```rust
pub async fn log_tool_call(...) -> Result<()>
```

Журналирует MCP-вызовы с latency (миграция `011_audit_log.sql`). Сейчас пишется, но публичных handler'ов на чтение нет — заглядывай SQL'ем напрямую.

## LanceStorage

Поменьше API: основные операции — upsert чанков с эмбеддингами и поиск.

```rust
pub async fn new(db_path: &str, vector_dim: usize) -> Result<Self>
pub async fn health_check(&self) -> Result<()>
pub async fn count(&self) -> Result<usize>
```

`vector_dim` фиксирует размерность; должна совпадать с `[embedding].dimensions`.

### Запись

```rust
pub async fn upsert_chunks(&self, chunks: &[CodeChunk], embeddings: &[Vec<f32>]) -> Result<()>
pub async fn delete_file(&self, file_path: &str) -> Result<()>
```

`upsert_chunks` ожидает `chunks.len() == embeddings.len()`. `delete_file` снимает все чанки файла (используется при удалении/перемещении).

### Поиск

```rust
pub async fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<SearchHit>>
pub async fn search_with_filter(...) -> Result<Vec<SearchHit>>
```

`SearchHit` — внутренняя структура с `chunk_id`, `score`, `content`, `file_path`, `line_range`. На MCP-уровне маппится в `SearchResult` (см. [models.md](models.md)).

`search_with_filter` принимает SQL-like фильтр по полям (например, `domain = 'backend' AND language = 'rust'`).

### Обслуживание

```rust
pub async fn create_vector_index_incremental(...) -> Result<()>
pub async fn compact(&self) -> Result<()>
```

`compact` объединяет фрагменты — вызывается автоматически в конце pipeline'а. Если делаешь много мелких инкрементов (watcher активен на проекте с частыми правками), периодически `compact` вручную поможет с read-amplification.

`create_vector_index_incremental` строит ANN-индекс. Ленив: первый поиск без индекса — медленный, после построения — быстрый.

## Пример: прямой доступ из скрипта

Если хочется лазить в SQLite напрямую (минуя MCP):

```bash
sqlite3 /path/to/project/.gofer/data/index.sqlite "
  SELECT s.name, s.kind, f.path, s.line_start
  FROM symbols s JOIN files f ON f.id = s.file_id
  WHERE s.name LIKE 'tool_%'
  ORDER BY f.path, s.line_start
  LIMIT 50;
"
```

WAL-режим включён. Если демон активен, открывай `?mode=ro` чтобы не словить блокировок:

```bash
sqlite3 "file:./.gofer/data/index.sqlite?mode=ro&immutable=0" -cmd ".timeout 1000" ...
```

LanceDB без CLI: придётся ходить через Rust/Python LanceDB SDK на ту же папку `.gofer/data/lance/`. Прямой shell-доступ к векторам не предусмотрен.

## Где смотреть в коде

- `src/storage/sqlite.rs` — все методы и их SQL-внутренности.
- `src/storage/lance.rs` — LanceDB обвязка.
- `src/storage/mod.rs` — re-exports.
- `migrations/` — реальные определения таблиц (см. [architecture.md](architecture.md#миграции-sqlite)).
