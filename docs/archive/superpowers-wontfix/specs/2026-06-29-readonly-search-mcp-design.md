# gofer → read-only index-search MCP

**Дата:** 2026-06-29
**Статус:** утверждён, ожидает плана реализации
**Ветка:** feat/code-intel-tools

## Цель

Превратить gofer из MCP-сервера «поиск + рефакторинг + LSP» в **чистый
read-only язык-агностичный поисковик/навигатор по коду на собственном индексе**.

Убрать:
1. все инструменты, мутирующие код пользователя;
2. все инструменты, выполняющие произвольный код / тесты (sandbox + cargo-run);
3. весь языковой слой — LSP-интеграцию **и** язык-специфичные сервисы.

Итоговый gofer: tree-sitter парсинг → SQLite (символы/refs) + LanceDB (векторы) →
семантический + структурный поиск и навигация. **Ноль внешних процессов**, кроме
HTTP-эмбеддера (`indexer/embedder.rs`) и загрузки wasm-грамматик
(`parser/lang_manager`).

### Мотивация

1. **Фокус/позиционирование** — один чёткий продукт: лучший MCP для поиска и
   навигации по кодовой базе на индексе.
2. **Уход от дублирования** — мутации дублируют нативные инструменты хост-агента
   (Claude Code/Cursor); LSP-инструменты дублируют IDE/host; язык-сервисы частично
   дублируют нативные индексные инструменты.
3. **Безопасность/доверие** — read-only сервер без спавна внешних процессов
   (language servers, cargo/tsc/python/go) проще доверять.
4. **Меньше кода/поддержки** — суммарно удаляется ~11K строк (~31% базы).

## Сводный объём

| Блок удаления | Инструментов | Строк |
|---|---|---|
| Мутации кода + sandbox (фаза 1 проекта) | ~25 | ~3 000 |
| Языковой слой: LSP + язык-сервисы (вариант B) | ~51 | ~7 900 |
| **Итого** | **~76** | **~11 000** |

**Было:** 87 core-инструментов + 37 язык-сервисных = 124.
**Станет:** ~47 read-only инструментов (индекс/git/чтение/поиск).

Архитектурный факт (подтверждён анализом): нативная навигация и поиск
(`get_symbols`, `get_references`, `get_callers`, `get_callees`,
`find_implementations`, `call_path`, `search`, `search_symbols`) и
индексатор/парсер **не зависят от LSP и язык-сервисов** — работают на собственном
индексе (SQLite + LanceDB + tree-sitter). Удаление языкового слоя их не ломает.

---

## Блок 1. Мутации кода + выполнение кода

### Удаляются (25 инструментов)

**Файлы хендлеров целиком:**

| Файл | Инструменты |
|---|---|
| `handlers/transactions.rs` (622) | begin_transaction, add_operation, commit_transaction, rollback_transaction, list_transactions |
| `handlers/trash.rs` (362) | delete_safe, list_trash, restore, purge_trash |
| `handlers/cas_buffer.rs` (525) | clipboard_copy, clipboard_paste, clipboard_replace, clipboard_store_text, clipboard_list, clipboard_clear |
| `handlers/sandbox.rs` (557) | execute_code, execute_function, run_test, run_all_tests |

**Частичная правка (удалить только перечисленные функции):**

| Файл | Удалить | Оставить |
|---|---|---|
| `file_ops.rs` (469) | write_file, patch_file, append_to_file, create_directory, move_file | list_directory, get_file_metadata |
| `code_quality.rs` (705) | format_file, apply_lint_fix | lint_file |
| `project.rs` (235) | add_rule, mark_golden_sample | get_dependencies, dependency_impact, project_tree, domain_stats, get_vue_tree |

> `lsp_rename` тоже мутация, но он уходит вместе со всем LSP в Блоке 2.

### Решения по пограничным зонам

- **Sandbox/выполнение кода** — удалить целиком (строго read-only).
- **CAS-буфер (clipboard)** — удалить целиком (помощник переноса кода = рефакторинг).
- **add_rule / mark_golden_sample** — удалить (запись в БД gofer).
- **force_reindex** — **оставить** (операционное управление индексом).
- **verify_patch** — **оставить** (валидация патча без применения, read-only).
- **batch_operations** — **оставить без изменений**: уже read-only (батчит только
  read_file, get_symbols, search, skeleton, get_references, read_function_context,
  read_types_only).

---

## Блок 2. Языковой слой (LSP + язык-сервисы) — вариант B

Удаляется **весь** каталог `src/languages/` и связанная инфраструктура. gofer
становится язык-агностичным.

### Удаляются (~51 инструмент, ~7 900 строк)

**Core LSP-инструменты** (`handlers/lsp.rs`, 785) — 14 шт:
lsp_goto_definition, lsp_find_references, lsp_hover, lsp_diagnostics,
lsp_completions, lsp_inlay_hints, lsp_code_actions, lsp_document_symbols,
lsp_workspace_symbols, lsp_goto_implementation, lsp_rename, lsp_expand_macro,
lsp_incoming_calls, lsp_outgoing_calls.

**Мета-инструменты** (`handlers/lang_tools.rs`, 203) — 2 шт:
lang_tools_list, lang_tools_call.

**Язык-сервисы** (`src/languages/`, 6 888) — ~35 шт:

| Файл | Строк | Инструменты |
|---|---|---|
| `generic_lsp.rs` | 861 | (нет инструментов — транспортный LSP-клиент) |
| `rust.rs` | 855 | rust_project_info, rust_expand_macro, rust_explain_struct, rust_find_trait_impls, rust_resolve_module_path, rust_check_code, rust_clippy, rust_test_run + 7 LSP-проксей |
| `typescript.rs` | 1661 | ts_inspect_type, ts_get_signature, ts_get_exports, ts_resolve_import, ts_check_file, ts_find_references |
| `vue.rs` | 1427 | vue_get_meta, vue_find_usages, vue_check_template, vue_get_props, vue_find_components |
| `python.rs` | 1393 | py_inspect_signature, py_find_definitions, py_find_imports, py_check_syntax |
| `go.rs` | 649 | go_list_modules, go_find_usages, go_analyze_interface, go_test_coverage, go_resolve_import, go_check_syntax |

> Консистентность: `rust_test_run` (и `rust_clippy`/`rust_check_code`) спавнят
> cargo — то же «выполнение кода», что и удаляемый sandbox. Уходят заодно.

### Что заменяет утрату

Потеря язык-фиделити (тип-инспекция TS, vue-props, py-сигнатуры и т.д.)
компенсируется нативными индексными инструментами: индекс недавно обогащён
сигнатурами и аннотациями (коммит a73b885), `get_symbols`/`search_symbols`/
`structural_search`/`find_by_type_signature` покрывают значительную часть
язык-агностично.

### Точки сцепления для зачистки

- `handlers/mod.rs` — убрать `pub mod lsp; pub mod lang_tools;`.
- `src/languages/` — удалить каталог целиком; убрать `mod languages;` (или
  эквивалент) в корневом модуле.
- `daemon/tools.rs` — удалить 14 веток `lsp_*` + 2 `lang_tools_*` в `dispatch()`
  и их `json!()` блоки в `core_tools_list()`.
- `daemon/state.rs` — удалить поле `language_services` (line 223), его
  инициализацию (line 417) и функцию `init_language_services()` (lines 815-844).
- `handlers/common.rs` (`ToolContext`) — удалить поля `language_services`,
  `lsp_clients` и метод `get_lsp_client()` (lines 31-157).
- `ipc/server.rs` — удалить мёрдж язык-сервисных схем в `tools/list`
  (lines 615-680, цикл по `project.language_services`), динамическую
  диспетчеризацию язык-инструментов и передачу `language_services`/`lsp_clients`
  в `ToolContext` (lines 675, 814, 970).

### Зависимости (Cargo.toml)

- `lsp-types` — удалить (LSP-только).
- `shell_words`, `which` — использовались только в `sandbox.rs`, `common.rs`
  (get_lsp_client) и `state.rs` (init язык-сервисов) — после удаления станут
  лишними, убрать.
- `reqwest` — **оставить** (используется эмбеддером `indexer/embedder.rs` и
  загрузчиком wasm-грамматик `parser/lang_manager`).

---

## Остаётся (read-only ядро, ~47 инструментов)

- **Поиск:** search, search_by_purpose, smart_file_selection, structural_search
- **Символы/граф вызовов:** get_symbols, get_references, get_callers, get_callees,
  call_path, dependency_subgraph, find_implementations, find_unused_symbols,
  find_by_type_signature, search_symbols, symbol_exists, is_exported
- **Чтение файлов:** read_file, skeleton, read_function_context, read_types_only,
  context_bundle, find_files, grep, find_unused_imports, file_exists,
  list_directory, get_file_metadata
- **Git (read-only):** git_blame, git_history, git_diff, suggest_commit, verify_patch
- **Проект:** project_tree, get_dependencies, dependency_impact, domain_stats,
  get_vue_tree
- **Анализ качества (read-only):** complexity, find_unreachable, lint_file
- **Диагностика/индекс:** run_diagnostics, health_check, get_config_keys,
  has_tests_for, get_index_status, validate_index, get_cache_stats,
  get_query_stats, force_reindex
- **Батч:** batch_operations (read-only)

---

## Подход: поэтапное удаление (вариант C)

На каждом шаге `cargo build` зелёный.

### Фаза 1 — снять мутации/sandbox с поверхности
`tools.rs`: удалить ветки `match` Блока 1 в `dispatch()` + их `json!()` блоки в
`core_tools_list()`. Инструменты исчезают из `tools/list`.

### Фаза 2 — снять языковой слой с поверхности
`tools.rs`: удалить 14 `lsp_*` + 2 `lang_tools_*` ветки и схемы.
`ipc/server.rs`: убрать мёрдж и диспатч язык-сервисных инструментов
(lines 615-680). После этого `tools/list` отдаёт только read-only ядро.

### Фаза 3 — удалить код
- `handlers/mod.rs`: убрать `transactions, trash, cas_buffer, sandbox, lsp,
  lang_tools`.
- Удалить файлы: `transactions.rs`, `trash.rs`, `cas_buffer.rs`, `sandbox.rs`,
  `lsp.rs`, `lang_tools.rs` и каталог `src/languages/` целиком.
- Вырезать частичные функции из `file_ops.rs`, `code_quality.rs`, `project.rs`.
- `state.rs`: убрать `language_services` (поле/init/функция).
- `common.rs` (`ToolContext`): убрать `language_services`, `lsp_clients`,
  `get_lsp_client()`.
- `Cargo.toml`: убрать `lsp-types`, `shell_words`, `which`.
- Зачистить мёртвый код по предупреждениям компилятора (импорты, helpers).

### Фаза 4 — зачистка
- Документация: `README`, `docs/desc/OVERVIEW.md`, описание сервера в `.mcp.json`
  → позиционирование «read-only index-search/navigation MCP».
- Память проекта (`project_gofer.md`).
- **Опционально:** миграция `018_drop_unused.sql` — drop таблиц trash,
  transactions, cas_buffer, golden_samples, audit_log (если относятся только к
  удалённым подсистемам). Миграции append-only — 001–017 не трогаем; неиспользуемые
  таблицы безвредны, drop — косметика.

---

## Тестирование и верификация

После **каждой** фазы:
- `cargo build` — зелёный (фазы 1–2: возможны предупреждения о неиспользуемом
  коде, это нормально; фазы 3–4: без предупреждений о мёртвом коде удалённых
  модулей).
- `cargo test` — существующие `#[cfg(test)]` тесты проходят (тесты внутри
  удаляемых модулей удаляются вместе с ними).

MCP smoke-тест (после фазы 2 и финально):
- `tools/list` отдаёт ~47 инструментов (не 124).
- Вызов удалённого инструмента (`write_file`, `lsp_hover`, `ts_inspect_type`,
  `run_test`) → `MethodNotFound`.
- `search`, `skeleton`, `get_callers`, `get_references`, `read_function_context`
  работают как раньше (на индексе).
- Индексация проекта (Rust/TS/Vue/Python/Go) по-прежнему работает через
  tree-sitter — язык-агностичный парсинг не затронут.

---

## Открытые вопросы (решить при реализации)

1. **Состояние демона** — кроме `init_language_services`, проверить нет ли иной
   инициализации, завязанной на язык-сервисы/LSP при старте (например, прогрев
   LSP-клиентов, фоновые задачи). Убрать если есть.
2. **`get_vue_tree` / `domain_stats`** — оставляем (read-only проектная
   аналитика на индексе). ✅ Проверено: `get_vue_tree` читает из
   `ctx.sqlite.get_vue_tree(...)` (собственный индекс), не из `VueService` —
   от удаляемого `languages/` не зависит. Блокера нет.
3. **Миграция `018_drop_unused.sql`** — делать ли drop неиспользуемых таблиц или
   оставить их как безвредный артефакт. Решение можно отложить до конца фазы 4.
