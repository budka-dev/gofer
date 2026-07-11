# Roadmap и статус фич

Срез текущего состояния gofer'а. Этот документ — честная сводка «что работает», «что наполовину», «что в архиве идей». Если в коде что-то поменялось — этот файл может отставать; авторитет — `git log` и `src/daemon/tools.rs::dispatch`.

Дата среза: **2026-07** (surface = **16 read-only index-search tools**; см. `tools.rs::dispatch`).

## Уровни статуса

- ✅ **Готово** — фича работает и доступна через MCP/CLI.
- 🟡 **Частично** — фича частично реализована или работает с ограничениями (описаны).
- 🔵 **В планах** — есть дизайн в `docs/archive/`, но кода нет либо он на ранней стадии.
- ❌ **Удалено / не делается** — снято с surface или решение «не нужно».

## Phase 0: Foundation

Базовый стек. Многие бывшие MCP tools сняты при переходе на read-only index-search model.

| Фича | Статус | Комментарий |
|---|---|---|
| 001 `get_index_status` | ✅ | Completeness, sync age, embedder probe. |
| 002 `validate_index` | ✅ | Integrity issues + recommendations. |
| 003 `reindex` (ex `force_reindex`) | ✅ | `path` = one file; `force=true` = clear + full_sync + resolve. |
| 004 `skeleton` | ✅ | AST-based signatures without bodies. |
| 005 Lightweight checks (`symbol_exists`, …) | ❌ | Сняты с dispatch; host FS / symbol tools. |
| 006 `search` + `include_scores` | ✅ | Hybrid vector+FTS; scores optional. |
| 007 `suggest_commit` | ❌ | Снят; commit — host git. |
| 008 Server-side cache | 🟡 | LRU `CacheManager` в `cache.rs`; MCP `get_cache_stats` снят. |
| 009 `read_function_context` | 🟡 | Работает; `include_callees=true` — 1 уровень. |
| 010 `read_types_only` | ✅ | Фильтрация по `kind`, docs. |
| 011 `smart_file_selection` / `scoring_index` | ❌ | Сняты; используй `search`. |
| 012 Incremental indexing | ✅ | `chunk_cache` + watcher. |
| 013 `batch_operations` | ✅ | search / get_symbols / skeleton / get_references / read_function_context / read_types_only. |
| 014 Query optimization | 🟡 | SQL-индексы `014_*.sql`; MCP `get_query_stats` снят. |
| 015 Connection pooling | ✅ | `ResourceLimits` + semaphore. |
| 016 Error recovery | ✅ | Circuit breakers embedding/vector. |

**Актуальный MCP surface:** ~16 tools — search, symbols/refs/callers/impls, compact read, batch, index ops. Канон: [tools-reference.md](tools-reference.md).

## Phase 1: Runtime Context

Идея — связать статический код с runtime-наблюдениями (тесты, профили, логи).

| Фича | Статус | Комментарий |
|---|---|---|
| `get_test_coverage` | 🔵 | В плане. Требует интеграции с tarpaulin/llvm-cov. |
| `get_runtime_examples` | 🔵 | В плане. Идея — собирать input/output из тестов и stash в knowledge-base. |
| `get_performance_hotspots` | 🔵 | В плане. Нужна интеграция с profilers (samply, flamegraph). |
| `find_error_patterns` | 🔵 | В плане. Лог-анализ + panics из CI. |
| `get_code_evolution` | 🔵 | В плане. Эвристика на основе git history. |
| `find_hotspots` | 🔵 | В плане. Совмещение `code_churn` + сложности. |
| `find_all_todos` | 🔵 | Host `rg` / agent; gofer grep tool снят. |
| `get_code_churn` | 🔵 | В плане. |
| `analyze_uncommitted_changes` | 🔵 | Host `git`; gofer `git_diff` снят. |
| `suggest_tests_for_changes` | 🔵 | В плане. |
| `check_breaking_changes` | 🔵 | В плане. Семантический diff API. |
| `get_symbol_context` | 🟡 | Эмулируется через `read_function_context` + `get_callers/callees`. |
| `smart_context_bundle` | 🟡 | Эмулируется через `context_bundle skeleton_deps_only=true`. |

## Phase 2: Production Intel

Идея — добавить организационный/исторический контекст: кто писал, почему, какие были обсуждения.

| Фича | Статус | Комментарий |
|---|---|---|
| `get_code_owners` | 🔵 | В плане. Гit-blame + CODEOWNERS. |
| `get_design_decisions` | 🔵 | В плане. ADR/RFC интеграция. |
| `get_related_discussions` | 🔵 | В плане. GitHub/GitLab API. |
| `search_similar_problems` | 🔵 | В плане. Поверх knowledge-base. |
| `search_logs` | 🔵 | В плане. Интеграция с Loki/Elasticsearch. |
| `find_production_errors` | 🔵 | В плане. Sentry/Bugsnag. |
| `get_function_metrics` | 🔵 | В плане. APM-провайдеры. |

## Phase 3+: Удалённые направления

Следующие блоки были частично или полностью реализованы, но **удалены** в ходе рефакторинга на read-only модель (2026-06-30). Функциональность доступна через хост-агента (Claude Code, Qoder и т. д.).

| Фича | Статус |
|---|---|
| **Atomic transactions** (`begin_transaction` / `add_operation` / `commit_transaction` / `rollback_transaction` / `list_transactions`) | ❌ Удалено |
| **CAS-буфер** (`clipboard_*`) | ❌ Удалено |
| **Sandbox** (`execute_code`, `execute_function`, `run_test`, `run_all_tests`) | ❌ Удалено |
| **Code quality** (`format_file`, `lint_file`, `apply_lint_fix`) | ❌ Удалено |
| **Trash** (`delete_safe`, `restore`, `list_trash`, `purge_trash`) | ❌ Удалено |
| **File ops** (`write_file`, `append_to_file`, `patch_file`, `create_directory`, `move_file`) | ❌ Удалено |
| **LSP-инструменты** (`lsp_hover`, `lsp_goto_definition`, …) | ❌ Удалено |
| **Language-tools** (`lang_tools_call`, `rust_*`, `ts_*`, `vue_*`, `py_*`, `go_*`) | ❌ Удалено |

## Инфраструктура

| Фича | Статус | Комментарий |
|---|---|---|
| Daemon + Unix socket | ✅ | `daemonize`, JSON-RPC 2.0. |
| MCP stdio bridge | ✅ | `gofer mcp`. |
| Prometheus метрики | ✅ | `127.0.0.1:9091/metrics`. |
| Структурированное логирование | ✅ | `tracing` + `tracing-appender`. См. [logging.md](logging.md). |
| Graceful shutdown | ✅ | SIGTERM/SIGINT → CancellationToken. |
| Auto-restart демона | ❌ | Не делается. Запуск через `gofer up` руками или через системный сервис. |
| Multi-host federation | ❌ | Не делается. Один демон = одна машина. |
| Hot-reload конфига | ❌ | Не делается. Перезапуск демона. |
| HTTP transport (вместо socket) | ❌ | Не делается. JSON-RPC через `nc -U` работает скриптам. |

## Хранилище и индекс

| Фича | Статус | Комментарий |
|---|---|---|
| SQLite метаданные | ✅ | Миграции в `migrations/`. |
| LanceDB векторы | ✅ | С компакцией после pipeline. |
| rkyv hot scoring index | ❌ | `scoring_index.rs` удалён; rkyv остаётся для embedding blobs + symbol cache. |
| chunk_cache для skip-unchanged | ✅ | По `content_hash`. |
| Watcher (notify + debounce) | ✅ | 500 мс окно. |
| Reranker | 🔵 | В планах (не в текущем hybrid search). |
| Domain detection (config-driven) | 🟡 | Эвристики path/content; см. `indexer/domains.rs` / config-reference. |
| Cross-stack links | 🟡 | Таблицы миграций могут заполняться; отдельного MCP reader tool нет. |

## Дропнутые направления

Эти идеи фигурировали в архиве, но решение «не делать»:

- **Локальный эмбеддер (ort/onnx/BGE)** — был, выпилили. Причины — в [faq.md::Зачем внешний эмбеддер](faq.md#зачем-внешний-эмбеддер-не-проще-ли-локально).
- **File summaries** (LLM-генерируемые резюме файлов для семантического поиска) — миграция `006_summaries.sql` создавала таблицы, миграция `017_drop_summaries.sql` их удалила. Идея признана непрактичной (стоимость генерации vs выгода).
- **Windows-поддержка** — Unix-socket в `daemonize`. Перейти на named pipes — большая работа без явного спроса.

## Что архив всё ещё содержит интересного

`docs/archive/next_stage/`:

- `ROADMAP.md` — стратегические направления Phase 1+ (откуда взяты пункты выше).
- `ROADMAP_EXTENSIONS.md`, `ROADMAP_INFRASTRUCTURE.md`, `ROADMAP_SANDBOXES.md` — детализация.
- `OPTIMIZATION_OPPORTUNITIES.md` — список potential оптимизаций.
- `SMART_COMMIT_DESIGN.md` — дизайн `suggest_commit` (**HISTORICAL**; tool снят).
- `IMPLEMENTATION_PLAN.md` — общий план.

Эти документы — **исторические**: код мог уйти вперёд или в сторону. Используй их как источник идей, не как спецификацию.

`docs/archive/desc/phase-{0,1,2}/` — спеки отдельных фич. Тоже исторические.

## Как помочь

Если хочешь поработать над «🔵 В планах» — открывай issue с обсуждением, что именно собираешься делать. Рецепт добавления — в [development.md::Добавление MCP-инструмента](development.md#добавление-mcp-инструмента-рецепт).

Если нашёл, что что-то «🟡 Частично» уже сделано полностью или наоборот — PR на правку этого документа.
