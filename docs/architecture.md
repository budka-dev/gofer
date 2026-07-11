# Архитектура gofer

Этот документ описывает, как устроен gofer внутри: какие процессы запускаются, как они общаются, как файл проекта превращается в индексированный чанк с эмбеддингом, и какие компоненты обслуживают MCP-запрос.

Документ соответствует состоянию репозитория на момент написания. Точные пути файлов указаны для навигации, но при сомнениях проверяйте `git log`.

## Карта процессов

gofer — это один бинарь с несколькими режимами работы:

```
┌────────────────────┐        Unix socket          ┌──────────────────────┐
│  gofer mcp (stdio) │ ───── ~/.gofer/daemon.sock ─►│   gofer daemon       │
│  bridge для MCP    │                              │   (демонизированный  │
└────────────────────┘                              │    процесс)          │
        ▲                                           │                      │
        │ stdio JSON-RPC                            │  ┌────────────────┐  │
        ▼                                           │  │ DaemonState    │  │
┌────────────────────┐                              │  │ projects, pools│  │
│  MCP-клиент        │                              │  └────────────────┘  │
│  (Claude Code,     │                              │                      │
│   Qoder, ...)      │      gofer up / status /     │  HTTP :9091          │
└────────────────────┘      reindex / search →      │  Prometheus          │
                            DaemonClient → socket   └──────────────────────┘
```

- `gofer daemon` запускается через `daemonize::Daemonize` (см. `src/main.rs::run_daemon`). Логи уходят в `~/.gofer/daemon.log` (stdout) и `daemon.err` (stderr), PID — в `daemon.pid`.
- Все остальные CLI-команды и `gofer mcp` подключаются к демону через `ipc::client::DaemonClient` по Unix-сокету. Если демона нет, вызывается `ensure_daemon_running()` — он его поднимает.
- Tokio runtime в демоне настраивается отдельно: `max(num_cpus/2, 4)` worker threads, `num_cpus` blocking threads (`src/main.rs:203`).
- Глобальный аллокатор — jemalloc (`tikv_jemallocator`).

## Слои кода (`src/`)

| Слой | Назначение | Ключевые файлы |
|---|---|---|
| `main.rs` | CLI (clap), демонизация, helper `ensure_daemon_running`. | `main.rs` |
| `daemon/` | Состояние демона, IPC-сервер, маршрутизатор инструментов, HTTP-метрики. | `daemon/state.rs`, `daemon/tools.rs`, `daemon/metrics_http.rs` |
| `daemon/handlers/` | Реализации MCP-инструментов (16 index-search tools). | `handlers/{files,search,symbols,index,batch,common}.rs` |
| `ipc/` | JSON-RPC поверх Unix-сокета: `server`, `client`, `protocol`, `bridge` (stdio↔socket). | `ipc/server.rs`, `ipc/bridge.rs` |
| `indexer/` | Пайплайн индексации, watcher, эмбеддер, domains. | `indexer/pipeline.rs`, `indexer/service.rs`, `indexer/watcher.rs`, `indexer/embedder.rs` |
| `indexer/parser/` | tree-sitter обвязка: динамический менеджер языков, chunking, skeleton extraction. | `parser/core.rs`, `parser/lang_manager/`, `parser/chunking.rs`, `parser/skeleton.rs` |
| `storage/` | Метаданные в SQLite (sqlx), векторы в LanceDB. | `storage/sqlite.rs`, `storage/lance.rs` |
| `cache.rs` | Серверный LRU-кеш ответов на инструменты (в т.ч. rkyv symbol cache). | `cache.rs` |
| `error_recovery.rs` | Circuit breakers для эмбеддера и векторного поиска. | `error_recovery.rs` |
| `resource_limits.rs` | Семафоры и rate limits для соединений и запросов. | `resource_limits.rs` |
| `logger.rs` | Инициализация `tracing` для трёх ролей (cli, mcp, daemon). | `logger.rs` |

Полный список миграций SQLite — в `migrations/001_*.sql` … `017_drop_summaries.sql`.

## DaemonState и проекты

`src/daemon/state.rs::DaemonState` — единственный долгоживущий объект внутри демона. Держит:

- `gofer_home: PathBuf` — глобальный каталог `~/.gofer/`.
- `registry: RegistryDb` — список зарегистрированных проектов (отдельная SQLite).
- `projects: RwLock<HashMap<String, Arc<ProjectState>>>` — активные проекты, ключ — UUID.
- `sync_progress: Arc<SyncProgress>` — снапшот текущей индексации (читается из `daemon/sync_progress`).
- `shutdown_token: CancellationToken` — глобальный сигнал на graceful shutdown (SIGTERM/SIGINT).
- `connection_semaphore: Arc<Semaphore>` — лимит одновременных подключений к сокету.
- `metrics: Arc<DaemonMetrics>` — lock-free счётчики (см. `state.rs:55+`).
- `notify_tx: broadcast::Sender<String>` — канал для server-to-client уведомлений (например, `notifications/tools/list_changed`).
- `resource_limits: Arc<ResourceLimits>` — rate-limit / throttle.
- `embedding_circuit`, `vector_circuit: Arc<CircuitBreaker>` — circuit breakers по двум критическим зависимостям.
- `lang_manager: Arc<LanguageManager>` — реестр tree-sitter wasm-грамматик.

Проект (`ProjectState`) внутри хранит свои `SqliteStorage`, `LanceStorage`, `EmbedderPool`, `IndexerService`, watcher, lang_manager и handle на фоновой sync.

### Жизненный цикл проекта

```
gofer init       → daemon/register_project       → registry.insert
gofer start      → daemon/activate_project       → ProjectState::activate
                   ├─ open SqliteStorage + LanceStorage
                   ├─ запустить full_sync (pipeline)
                   └─ опционально watcher (notify + debouncer)
gofer sleep      → daemon/deactivate_project     → остановить watcher, освободить ProjectState
gofer down       → daemon/shutdown               → cancel + drop всех проектов
```

## IPC: как ходят запросы

### CLI и `gofer mcp` (клиент)

`ipc::client::DaemonClient` подключается к `~/.gofer/daemon.sock`, шлёт `DaemonRequest` (JSON-RPC: `id`, `method`, `params`), читает `DaemonResponse`. Протокол описан в `src/ipc/protocol.rs`.

### Сервер

`ipc::server::run_daemon` слушает сокет, на каждое соединение спавнит `handle_connection`. Маршрутизация запросов — в `handle_request` (`src/ipc/server.rs:312`). Распределение методов:

| Метод | Назначение |
|---|---|
| `daemon/status`, `daemon/health`, `daemon/metrics`, `daemon/shutdown` | Управление жизненным циклом. |
| `daemon/register_project`, `daemon/activate_project`, `daemon/deactivate_project` | Реестр проектов. |
| `daemon/sync_progress` | Снапшот текущей индексации (используется CLI прогресс-баром). |
| `tools/list`, `tools/call` | MCP-инструменты — см. ниже. |
| `resources/list`, `resources/read` | MCP-ресурсы. |
| `prompts/list` | MCP-промпты. |
| `reindex` | Триггер переиндексации. |

### Bridge: stdio ↔ socket

`ipc::bridge::run_bridge` (вызывается из `gofer mcp`) поднимает асинхронный мост:

- Читает JSON-RPC от MCP-клиента из stdin.
- Преобразует в `DaemonRequest` и шлёт демону через `DaemonClient`.
- Возвращает ответ обратно в stdout.
- Параллельно слушает broadcast-уведомления демона (`tools/list_changed`) и проксирует их клиенту.

Никакой обработки инструментов в bridge нет — только трансляция. Это позволяет одному демону обслуживать несколько MCP-клиентов одновременно.

## Sequence: tools/call от клиента до ответа

```
 MCP-client     gofer mcp (bridge)     daemon socket       handle_tools_call
 ──────────    ──────────────────     ──────────────       ──────────────────
     │                │                       │                       │
     │ stdin: {"method":"tools/call","name":"search",...}              │
     ├───────────────►│                       │                       │
     │                │ DaemonRequest         │                       │
     │                │  + project_path=cwd   │                       │
     │                ├──────────────────────►│                       │
     │                │                       │ handle_request        │
     │                │                       ├──────────────────────►│
     │                │                       │                       │
     │                │                       │     get_or_load_project (lazy)
     │                │                       │                       │
     │                │                       │     build ToolContext:
     │                │                       │       sqlite, lance, embedder,
     │                │                       │       cache, circuit breakers
     │                │                       │                       │
     │                │                       │     tools::dispatch(name, args, ctx)
     │                │                       │       ── search::tool_search
     │                │                       │            ├─ embed_query (circuit-protected)
     │                │                       │            ├─ lance retrieval (circuit-protected)
     │                │                       │            ├─ BM25 + rerank scoring
     │                │                       │            └─ token-optimised content
     │                │                       │                       │
     │                │                       │ DaemonResponse{result}│
     │                │                       │◄──────────────────────┤
     │                │ JSON-RPC response     │                       │
     │                │◄──────────────────────┤                       │
     │ stdout: {"jsonrpc":"2.0","id":...,"result":{...}}              │
     │◄───────────────┤                       │                       │
     │                │                       │                       │
```

Параллельно демон может публиковать `notifications/*` через broadcast-канал (`tools/list_changed`, `notifications/progress` — см. протокол). Bridge их транслирует клиенту как обычные RPC-уведомления без `id`.

## Маршрутизация MCP-инструментов

Когда приходит `tools/call` с `name: "..."` и `arguments: {...}`, сервер вызывает `daemon::tools::dispatch(name, args, ctx)` (`src/daemon/tools.rs`). `ToolContext` собирается из текущего `ProjectState` (sqlite, lance, embedder, lsp manager, кеш, метрики) и передаётся в конкретный handler.

Handlers сгруппированы по семантическим доменам (`src/daemon/handlers/`). **Канон — `tools.rs::dispatch` + `core_tools_list` (~16 tools):**

| Группа | Файл | Инструменты |
|---|---|---|
| search | `search.rs` | `search` |
| symbols | `symbols.rs` | `search_symbols`, `get_symbols`, `get_references`, `get_callers`, `get_callees`, `find_implementations`, `find_by_type_signature` |
| files | `files.rs` | `skeleton`, `read_function_context`, `read_types_only`, `context_bundle` |
| index | `index.rs` | `get_index_status`, `validate_index`, `reindex` |
| batch | `batch.rs` | `batch_operations` |

FS/grep/git/LSP/mutation handlers сняты (read-only index-search model). Полный список — [tools-reference.md](tools-reference.md).

## Пайплайн индексации

Запускается из `ProjectState::activate` (полный sync) или из watcher'а (инкремент). Реализация — `src/indexer/pipeline.rs::run_pipeline`.

Это пятиступенчатый асинхронный pipeline с bounded-каналами для backpressure:

```
files ──▶ scanner ──▶ parser × N ──▶ batcher ──▶ embedder ──▶ writer
         (I/O)       (CPU)          (group)     (HTTP)       (sqlite+lance)
                                                                    │
                                                                    ▼
                                                          ParsedFileMetadata
                                                          (для пост-фаз: domains,
                                                           cross-stack links)
```

| Стейдж | Что делает | Параллелизм | Канал на выход |
|---|---|---|---|
| `scanner_stage` | Хеш-prefetch из SQLite, сравнение, отбрасывает unchanged. | 1 task | `ScannedFile` (буфер 512) |
| `parser_worker` | tree-sitter parse, извлечение Symbol/Reference/Import, определение domain. | `clamp(num_cpus/2, 4..=8)` | `ParsedDoc` (256) |
| `batcher_stage` | Группирует чанки в батчи по размеру/таймауту (`BATCH_TIMEOUT_MS = 50`, лимит 512 КБ). | 1 task | `ChunkBatch` (64) |
| `embedder_stage` | Шлёт батч во внешний эмбеддер, при простое сразу пишет в SQLite cache. | 1 task на пул | `EmbeddedBatch` (64) |
| `writer_stage` | Транзакционно пишет метаданные в SQLite, векторы в LanceDB, обновляет прогресс. | 1 task | — |

На входе и выходе:

- `EmbedderPool` скейлится до 4 инстансов на время sync, после — обратно до 1.
- В конце writer'а делается `lance.compact()` (уменьшение read-amplification).
- Cache eviction: max `100_000` записей и `30` дней (`pipeline.rs:243+`).
- При cancel'е (shutdown / `gofer sleep`) все стейджи завершаются через `CancellationToken`.

### Watcher и инкрементальная индексация

`src/indexer/watcher.rs` использует `notify` v7 + `notify-debouncer-mini`. События файловой системы дебаунсятся и попадают в `IndexerService::run`, который вызывает `index_file` для одиночного файла либо триггерит мини-pipeline.

Игноры читаются из `.gofer/config.toml` (`[indexer].ignore`) и поверх — `.gitignore` через crate `ignore`.

## Эмбеддер

`src/indexer/embedder.rs::EmbedderPool` — пул HTTP-клиентов. **Локальной модели нет**: gofer POSTит JSON `{ "texts": [...] }` на `external_url` (по умолчанию `http://127.0.0.1:8080/embed/`).

Ключевые методы:

- `embed(texts)` — батч эмбеддинг (используется в pipeline).
- `embed_query(text)` — единичный эмбеддинг для поисковых запросов.
- `scale_up(N)` / `scale_down(N)` — изменить размер пула на лету.
- `dimension()` / `model_name()` / `cache_version_key()` — метаинформация для инвалидации кеша.

URL и имя модели берутся из секции `[embedding]` проектного конфига (`embedding.external_url`, `embedding.model`). Размер пула — `embedding.pool_size`. Размер батча — `embedding.batch_size`.

Сбои HTTP-эмбеддера ловит `embedding_circuit: CircuitBreaker` (`src/error_recovery.rs`). После N подряд ошибок цепь размыкается на cooldown — поиск продолжает работать на BM25/символах, эмбеддинг возобновится автоматически.

## Хранилище

### SQLite (`src/storage/sqlite.rs`, ~2300 строк)

Используется `sqlx` 0.8 с runtime-tokio. Подключение — connection pool из `resource_limits`. Файл БД — `<project>/.gofer/data/index.sqlite`.

Что хранится:

- Метаданные файлов: путь, хеш (blake3), mtime, язык, размер.
- Символы (`Symbol`): имя, kind, диапазон, родитель, видимость, docstring.
- Ссылки (`SymbolReference`): caller→callee для построения графа вызовов.
- Импорты, domains, golden samples, rules, audit log, diagnostics cache.
- `chunk_cache`: SHA-кеш чанков для skip-unchanged (см. миграцию `010_chunk_cache.sql`).
- `indexing_status` и `language` (миграция `015`).

Миграции — в `migrations/` (17 шт). Применяются автоматически через `sqlx::migrate!` при открытии БД.

### LanceDB (`src/storage/lance.rs`, ~800 строк)

Колонночный векторный store на Apache Arrow. Файлы — `<project>/.gofer/data/lance/`.

Схема: `id` (string), `path`, `chunk_range`, `kind`, `embedding` (FixedSizeList<f32, D>), плюс служебные поля для скоринга и фильтрации.

После записи pipeline делает `compact()` (объединение мелких фрагментов). Индексы строятся лениво при первом запросе.

### ~~Hot scoring index (`src/scoring_index.rs`)~~ — removed

Отдельный rkyv mmap `scoring_index` / `smart_file_selection` **удалены** с surface. Hybrid ranking живёт в `handlers/search.rs` (vector + FTS + symbol boost). `rkyv` по-прежнему используется для embedding blobs в SQLite и LRU symbol cache (`cache.rs`).

## Мультиязычность через tree-sitter

gofer — язык-агностичный инструмент. Поддержка языков реализуется через tree-sitter wasm-грамматики (см. ниже). LSP-серверы и язык-специфичные сервисы не используются.

### Динамические грамматики tree-sitter

`indexer/parser/lang_manager/` — менеджер wasm-грамматик. Грамматики не вкомпилены в бинарь: они скачиваются из lang-hub (`gofer install-lang <name>`), кешируются в `~/.gofer/lang-hub/`, загружаются `tree-sitter` через wasm-feature. Это позволяет добавлять язык без пересборки gofer.

## Метрики и наблюдаемость

- `daemon/metrics_http.rs` поднимает HTTP-сервер на `127.0.0.1:9091/metrics` в Prometheus text exposition format. Прокидывает счётчики из `DaemonMetrics`.
- `DaemonMetrics::snapshot()` отдаёт JSON для `daemon/health` и `gofer status`.
- Логи в `tracing` с фильтром `EnvFilter`. Уровень настраивается флагом `--log-level`. Появляются в `~/.gofer/daemon.log` и `daemon.err`.

## Кеш ответов инструментов

`cache.rs::CacheManager` — серверный LRU. Кеширует ответы тяжёлых read-only инструментов (skeleton, context_bundle, read_function_context). Ключ — комбинация имени инструмента, аргументов и хеша файлов. Инвалидация — при изменении файла (через watcher) или при изменении модели эмбеддера (через `cache_version_key`).

## Graceful shutdown

При SIGTERM/SIGINT (`src/main.rs:234+`) демон:

1. Триггерит `state.shutdown_token.cancel()`.
2. Все pipeline-стейджи и watcher'ы получают cancel и завершаются.
3. IPC-сервер перестаёт принимать новые соединения, текущие — дослуживают.
4. ProjectState'ы закрываются (sqlx pool drain, LanceDB close).
5. Процесс выходит.

Этим же путём идёт `gofer down` (через `daemon/shutdown`).

## Протокол JSON-RPC

Демон говорит JSON-RPC 2.0 поверх Unix-сокета (`src/ipc/protocol.rs`). Формат каждого фрейма — одна JSON-строка с `\n` разделителем.

Запрос:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "tools/call",
  "params": {
    "project_path": "/abs/path/to/project",
    "name": "search",
    "arguments": { "query": "auth middleware" }
  }
}
```

Успешный ответ:

```json
{ "jsonrpc": "2.0", "id": 7, "result": { ... } }
```

Ответ с ошибкой:

```json
{ "jsonrpc": "2.0", "id": 7, "error": { "code": -32602, "message": "Missing project_path" } }
```

`project_path` — это inject от bridge'а (берётся из cwd процесса `gofer mcp` или из `--project-dir`). CLI-команды добавляют его сами через `DaemonClient`.

Уведомления от сервера к клиенту (`DaemonNotification`) — без `id`, шлются через broadcast-канал. Используются для `notifications/tools/list_changed` и `notifications/progress` (см. `protocol.rs::DaemonNotification::progress`).

Коды ошибок — стандартные JSON-RPC (`-32600` invalid request, `-32601` method not found, `-32602` invalid params, `-32000…-32099` server errors). `GoferError::into_rpc()` мапит внутренние ошибки на эти коды.

## MCP Resources и Prompts

Помимо `tools/*`, демон отвечает на стандартные MCP-эндпоинты (`src/ipc/server.rs`):

### `resources/list`

Возвращает четыре ресурса проекта:

| URI | mimeType | Что внутри |
|---|---|---|
| `project://context` | `application/json` | Rules, golden samples, зависимости (из SQLite-таблиц). |
| `project://tree` | `application/json` | `project_tree` глубиной 3. |
| `project://stats` | `application/json` | Кол-во файлов, символов, распределение по доменам. |
| `project://config` | `application/json` | Найденные конфиг-ключи (`get_config_keys`). |

### `resources/read`

`{uri}` → диспетчеризуется на соответствующий handler или `tools::dispatch`. Неизвестный URI отдаёт `-32602`.

### `prompts/list`

На момент написания — заглушка с пустым списком: gofer не выставляет именованные prompt-шаблоны.

## Маршрутизация по модулям-помощникам

### `error_recovery.rs` — Circuit Breaker

Состояния — `Closed`, `Open { opened_at }`, `HalfOpen`. Параметры подбираются на инстанс:

| Брейкер | failure_threshold | recovery_threshold | timeout |
|---|---|---|---|
| `embedding_circuit` | 5 | 2 | 30 с |
| `vector_circuit` | 3 | 1 | 10 с |

Логика (`CircuitBreaker::call`):

1. `Closed`: операция исполняется; на ошибку счётчик растёт, на успех — обнуляется. После `failure_threshold` подряд ошибок → `Open`.
2. `Open`: пока не прошёл `timeout`, запросы немедленно отвечают «Circuit breaker open». По истечении переходит в `HalfOpen`.
3. `HalfOpen`: пробные запросы. После `recovery_threshold` успехов → `Closed`. На ошибку — снова `Open`.

Поиск (`handlers/search.rs`) оборачивает оба критичных вызова: embed-query через `embedding_circuit`, retrieval из LanceDB — через `vector_circuit`. При размыкании поиск пытается graceful fallback (BM25/символы) либо возвращает RPC-ошибку.

### `resource_limits.rs`

`ResourceLimits::default()` сейчас держит `max_concurrent_requests = 1024`. Каждый MCP-запрос получает `RequestGuard`, который освобождается на drop. При исчерпании — `ResourceLimitError::TooManyRequests { current, max }`. Помимо этого `DaemonState::connection_semaphore` ограничивает 1024 одновременными tcp-соединениями (см. `ipc/server.rs:38`).

### ~~`scoring_index.rs` / `commit.rs`~~ — removed

Отдельный hot scoring index и MCP `suggest_commit` сняты с read-only surface. Commit messages — host `git` / agent. Ranking — в `handlers/search.rs`.

### `domains`

Файлы при индексации тегируются доменом (`backend`, `frontend`, `ops`, `shared`, `rs` и т. п.) по эвристикам в `indexer/domains.rs` (и опционально config). Cross-stack links (миграции `007_structural_links.sql`) могут заполняться при индексации; отдельного MCP tool (`get_api_routes` / `domain_stats`) в текущем dispatch **нет**.

## Миграции SQLite

Применяются автоматически при открытии `SqliteStorage` через `sqlx::migrate!`. Назначение каждой:

| Файл | Что добавляет |
|---|---|
| `001_init.sql` | Базовые таблицы: `files`, `symbols`, поля `domain`/`tech_stack` на `files`. |
| `002_references.sql` | `symbol_references` — граф вызовов. |
| `003_knowledge.sql` | `dependencies` (Cargo/npm) + `rules` (Knowledge Base). |
| `004_diagnostics.sql` | Кеш активных диагностик из `cargo check`/`tsc`. |
| `005_domains.sql` | Cross-stack linking метаданные поверх domain-колонок из 001. |
| `006_summaries.sql` | LLM/docstring-сводки файлов. **Дропнута миграцией 017.** |
| `007_structural_links.sql` | `type_fingerprints` (Jaccard-сравнение) + `cross_stack_links`. |
| `008_subprojects.sql` | `subprojects` — workspace members для монорепо. |
| `009_index_metadata.sql` | `index_metadata` — completeness, прогресс индекса. |
| `010_chunk_cache.sql` | `chunk_cache (content_hash → embedding)` для skip-unchanged. |
| `011_audit_log.sql` | `audit_log` — все MCP-вызовы с латентностью. |
| `012_golden_samples_unique.sql` | UNIQUE-индекс на `golden_samples.file_id` (фикс `ON CONFLICT`). |
| `014_query_optimization.sql` | Дополнительные индексы для горячих запросов. *(013 отсутствует — резерв.)* |
| `015_add_indexing_status_and_language.sql` | Колонки `indexing_status` (pending/in_progress/completed/failed) и `language` на `files`. |
| `016_rkyv_optimization.sql` | Перевод нескольких JSON-TEXT полей в BLOB для rkyv zero-copy. |
| `017_drop_summaries.sql` | Удаляет `file_summaries`/`summary_queue` (фича выключена). |

Файл `013_*.sql` отсутствует — пропуск номера. `sqlx::migrate!` не возражает; новых миграций добавлять следует от `018_*`.

## Watcher

`indexer/watcher.rs::start_watcher` использует `notify-debouncer-mini` с окном `Duration::from_millis(500)`. Подписывается на наблюдаемые каталоги в режиме `RecursiveMode::NonRecursive` — список путей собирает `find_watchable_dirs`, обходя дерево и пропуская:

- паттерны из `[indexer].ignore` в `.gofer/config.toml`,
- стандартные `.gitignore` через crate `ignore`,
- скрытые директории (`.git`, `.gofer/data/`).

После дебаунса события группируются и кидаются в `IndexerService::run` через `mpsc`. Если файл больше `MAX_FILE_SIZE_BYTES = 2 МБ` (`pipeline.rs:91`) — пропускается с предупреждением.

## Глоссарий

Краткий перевод терминов, которые часто встречаются в коде и документации.

| Термин | Что это |
|---|---|
| **chunk** | Кусок текста файла переменного размера, который эмбеддится отдельно. Чанкинг живёт в `parser/chunking.rs`. |
| **content_hash** | blake3-хеш содержимого чанка. Используется в `chunk_cache` для skip-unchanged. |
| **circuit breaker** | Защитный паттерн из `error_recovery.rs`: после N подряд ошибок временно блокирует вызовы критичной зависимости. Состояния `Closed`/`Open`/`HalfOpen`. |
| **context bundle** | Файл + рекурсивно его import-зависимости, сложенные в один ответ. Опционально с скелетизацией зависимостей. |
| **cross-stack links** | Связи между backend-эндпоинтом и frontend-вызовом, найденные по эвристике (структуры запросов, имена путей). Таблица из миграции `007`. |
| **dispatch** | Единая точка маршрутизации `tools/call` (`src/daemon/tools.rs::dispatch`). |
| **domain** | Логическая зона проекта: `backend`/`frontend`/`ops`/`shared`. Назначается файлу при индексации по эвристике пути. |
| **EmbedderPool** | Пул HTTP-клиентов для внешнего эмбеддера, с семафором ограничивающим параллелизм. |
| **incremental indexing** | Переиндексация только изменённых файлов; `reindex` без `force` / watcher. |
| **lang-hub** | Внешний репозиторий с wasm-грамматиками tree-sitter, который `gofer install-lang` качает. |
| **MCP** | Model Context Protocol — стандарт интеграции LLM-агентов с внешними инструментами. gofer выставляет себя как MCP-сервер. |
| **MCP bridge** | `gofer mcp` — процесс, транслирующий stdio JSON-RPC от клиента в сокет демона. |
| **pipeline** | Пятиступенчатый асинхронный конвейер индексации (`scanner → parser → batcher → embedder → writer`). |
| **ProjectState** | Долгоживущий объект демона на каждый активный проект: SQLite, LanceDB, embedder pool, watcher. |
| **rkyv** | Бинарный сериализатор с zero-copy десериализацией. Embedding blobs в SQLite + LRU symbol cache (`cache.rs`). |
| **skeleton** | Файл без тел функций — только сигнатуры, типы, doc-комментарии. |
| **sync** | Полный проход по проекту: scan → parse → index (`full_sync`). Запускается `gofer start` / `reindex force=true`. |
| **tool** | MCP-инструмент, выставленный через `tools/list`. У каждого есть JSON-Schema аргументов. |
| **ToolContext** | Структура с зависимостями для handler'а: `sqlite`, `lance`, `embedder`, `cache`, circuit breakers. |
| **watcher** | Поток на `notify` + `notify-debouncer-mini`, реагирующий на изменения файлов. Дебаунс 500 мс. |

## Лимиты и константы

Все захардкоженные параметры на одной странице — удобно для capacity planning.

### Pipeline и индексация

| Параметр | Значение | Где |
|---|---|---|
| Парсер-воркеры | `clamp(num_cpus/2, 4..=8)` | `pipeline.rs::run_pipeline` |
| Базовый размер батча чанков | 96 | `pipeline.rs::BATCH_CHUNK_SIZE_BASE` |
| Min/Max батч | 32 / 256 | `BATCH_CHUNK_SIZE_MIN/MAX` |
| Таймаут батчинга | 50 мс | `BATCH_TIMEOUT_MS` |
| Max content/batch | 512 КБ | `BATCH_MAX_CONTENT_BYTES` |
| Max размер файла для индекса | 2 МБ | `MAX_FILE_SIZE_BYTES` |
| SQLite flush batch | 100 | `SQLITE_FLUSH_SIZE` |
| Буферы каналов (scan → parse → batch → embed) | 512 / 256 / 64 / 64 | `pipeline.rs` |

### Кеш

| Параметр | Значение | Где |
|---|---|---|
| Cache: max записей | 100 000 (~150 МБ) | `pipeline.rs::CACHE_MAX_ENTRIES` |
| Cache: max возраст | 30 дней | `CACHE_MAX_AGE_DAYS` |

### Демон и IPC

| Параметр | Значение | Где |
|---|---|---|
| Tokio worker threads | `max(num_cpus/2, 4)` | `main.rs::run_daemon` |
| Tokio blocking threads | `num_cpus` | там же |
| Connection semaphore | 1024 | `daemon/state.rs:268` |
| ResourceLimits: concurrent requests | 1024 | `resource_limits.rs:19` |
| Broadcast channel capacity | 64 | `daemon/state.rs:244` |
| Prometheus HTTP | `127.0.0.1:9091` | `main.rs::run_daemon` |
| Таймаут запуска демона | 30 с | `ensure_daemon_running` |

### Circuit breakers

| Брейкер | failure_threshold | recovery_threshold | timeout |
|---|---|---|---|
| `embedding_circuit` | 5 | 2 | 30 с |
| `vector_circuit` | 3 | 1 | 10 с |

### Watcher

| Параметр | Значение | Где |
|---|---|---|
| Дебаунс событий | 500 мс | `watcher.rs::start_watcher` |
| Режим notify | `RecursiveMode::NonRecursive` | там же |
| Обязательные игноры | `.git`, `.gofer`, `.qoder`, `*.lock` | `watcher.rs::build_gitignore` |

### Эмбеддер

| Параметр | Значение | Где |
|---|---|---|
| HTTP timeout клиента | 60 с | `embedder.rs::Embedder::new` |
| Pool size: clamp | `1..=8` | `EmbedderPool::with_config` |
| Pool size: пик при индексе | 4 | `pipeline.rs::run_pipeline` |
| Pool size: после индекса | 1 | там же |
| Дефолтный URL | `http://127.0.0.1:8080/embed/` | `embedder.rs:34` |
| Дефолтные dimensions | 1024 | `EmbedderPool::with_config:157` |
| Дефолтная модель | `external_model` | там же:160 |

## Что почитать дальше

- [examples.md](examples.md) — реальные сценарии использования инструментов.
- [tools-reference.md](tools-reference.md) — полный каталог MCP-инструментов (core + lang-tools).
- [config-reference.md](config-reference.md) — полная схема `.gofer/config.toml`.
- [embedder.md](embedder.md) — HTTP-контракт эмбеддера.
- [development.md](development.md) — сборка, тесты, рецепт добавления нового инструмента/миграции.
- [troubleshooting.md](troubleshooting.md) — типовые проблемы и где смотреть логи.
- `src/daemon/tools.rs` — точка диспетчеризации и канонический JSON-Schema.
- `src/indexer/pipeline.rs` — устройство pipeline.
- `migrations/` — схема SQLite по версиям.
