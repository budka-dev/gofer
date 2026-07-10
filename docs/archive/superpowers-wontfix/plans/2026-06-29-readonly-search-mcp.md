# gofer → read-only index-search MCP — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Удалить из gofer все инструменты, мутирующие код / выполняющие код, и весь языковой слой (LSP + язык-сервисы), оставив чистый read-only язык-агностичный поисковик/навигатор на собственном индексе.

**Architecture:** Поэтапное удаление (spec вариант C). Сначала снимаем инструменты с MCP-поверхности (диспетчер + схемы), затем удаляем мёртвый код хендлеров и языкового слоя, затем правим зависимости и документацию. Нативная навигация/поиск работают на SQLite+LanceDB+tree-sitter и от удаляемого кода не зависят, поэтому каждый шаг оставляет рабочий бинарь.

**Tech Stack:** Rust 2021, tokio, sqlx (SQLite), lancedb, tree-sitter (wasm). Сборка `cargo build`, тесты `cargo test`.

**Спека:** `docs/superpowers/specs/2026-06-29-readonly-search-mcp-design.md`

## Global Constraints

- После **каждой** задачи `cargo build` обязан быть зелёным. Предупреждения о
  неиспользуемом коде на промежуточных задачах (1–5) допустимы; к концу задачи 6
  их быть не должно.
- После **каждой** задачи `cargo test` обязан проходить.
- Это работа на удаление: TDD-цикл «failing test → impl» неприменим. Роль теста
  играет верификация `cargo build` + `cargo test` (+ MCP smoke на задачах 2 и 7).
- Коммитить после каждой задачи. Сообщения коммитов на русском/английском в
  существующем стиле (`refactor: ...`, `chore: ...`).
- Ничего не добавляем (YAGNI). Только удаление + правка точек сцепления.
- Не трогаем существующие миграции 001–017 (append-only).

---

## File Structure (что меняется)

**Удаляются целиком:**
- `src/daemon/handlers/transactions.rs`, `trash.rs`, `cas_buffer.rs`, `sandbox.rs`,
  `lsp.rs`, `lang_tools.rs`
- `src/languages/` — весь каталог (`mod.rs`, `generic_lsp.rs`, `rust.rs`,
  `typescript.rs`, `vue.rs`, `python.rs`, `go.rs`)

**Правятся:**
- `src/daemon/tools.rs` — `dispatch()` + `core_tools_list()`
- `src/daemon/handlers/mod.rs` — объявления модулей
- `src/daemon/handlers/common.rs` — `ToolContext` (поля + `get_lsp_client`)
- `src/daemon/handlers/file_ops.rs`, `code_quality.rs`, `project.rs` — частичный вырез
- `src/daemon/state.rs` — `ProjectState` (поля `language_services`, `lsp_clients`,
  `init_language_services`)
- `src/ipc/server.rs` — мёрдж/диспатч язык-сервисов, сборка `ToolContext` (×3)
- `Cargo.toml` — зависимости
- `README`, `docs/desc/OVERVIEW.md`, `.mcp.json`, память проекта

---

## Task 1: Снять с поверхности мутации + sandbox

**Files:**
- Modify: `src/daemon/tools.rs` (`dispatch()` ~71-102, `core_tools_list()`)

**Interfaces:**
- Produces: `dispatch()` больше не маршрутизирует 25 инструментов Блока 1; они
  отдают `MethodNotFound`. Хендлер-функции пока остаются (станут unused).

- [ ] **Step 1: Удалить ветки `match` Блока 1 в `dispatch()`**

В `src/daemon/tools.rs` в функции `dispatch()` удалить строки (по именам инструментов):

```rust
// File Operations — удалить ТОЛЬКО эти 5 (list_directory и get_file_metadata ОСТАВИТЬ):
"patch_file" => file_ops::tool_patch_file(args, ctx).await,
"write_file" => file_ops::tool_write_file(args, ctx).await,
"append_to_file" => file_ops::tool_append_to_file(args, ctx).await,
"create_directory" => file_ops::tool_create_directory(args, ctx).await,
"move_file" => file_ops::tool_move_file(args, ctx).await,
// Trash — все 4:
"delete_safe" => trash::tool_delete_safe(args, ctx).await,
"list_trash" => trash::tool_list_trash(args, ctx).await,
"restore" => trash::tool_restore(args, ctx).await,
"purge_trash" => trash::tool_purge_trash(args, ctx).await,
// Transactions — все 5:
"begin_transaction" => transactions::tool_begin_transaction(args, ctx).await,
"add_operation" => transactions::tool_add_operation(args, ctx).await,
"commit_transaction" => transactions::tool_commit_transaction(args, ctx).await,
"rollback_transaction" => transactions::tool_rollback_transaction(args, ctx).await,
"list_transactions" => transactions::tool_list_transactions(args, ctx).await,
// Code Quality — удалить 2 (lint_file ОСТАВИТЬ):
"format_file" => code_quality::tool_format_file(args, ctx).await,
"apply_lint_fix" => code_quality::tool_apply_lint_fix(args, ctx).await,
// CAS Buffer — все 6:
"clipboard_copy" => cas_buffer::tool_extract_to_clipboard(args, ctx).await,
"clipboard_paste" => cas_buffer::tool_insert_clipboard(args, ctx).await,
"clipboard_replace" => cas_buffer::tool_replace_with_clipboard(args, ctx).await,
"clipboard_store_text" => cas_buffer::tool_content_to_clipboard(args, ctx).await,
"clipboard_list" => cas_buffer::tool_list_clipboards(args, ctx).await,
"clipboard_clear" => cas_buffer::tool_clear_clipboard(args, ctx).await,
// Sandbox — все 4:
"execute_code" => sandbox::tool_execute_code(args, ctx).await,
"execute_function" => sandbox::tool_execute_function(args, ctx).await,
"run_test" => sandbox::tool_run_test(args, ctx).await,
"run_all_tests" => sandbox::tool_run_all_tests(args, ctx).await,
// Project — удалить 2 (остальные project::* ОСТАВИТЬ):
"add_rule" => project::tool_add_rule(args, ctx).await,
"mark_golden_sample" => project::tool_mark_golden_sample(args, ctx).await,
```

> ОСТАВИТЬ: `list_directory`, `get_file_metadata`, `lint_file`, `force_reindex`,
> `verify_patch`, `batch_operations` и все остальные.

- [ ] **Step 2: Удалить соответствующие схемы в `core_tools_list()`**

В той же функции `core_tools_list()` удалить `json!({ ... })` объект для каждого
из 25 имён выше (искать по `"name": "write_file"`, `"name": "delete_safe"`, и т.д.).
Удалять весь объект целиком, включая завершающую запятую.

- [ ] **Step 3: Сборка**

Run: `cargo build 2>&1 | tail -20`
Expected: SUCCESS. Возможны warnings `function is never used` для удалённых
хендлеров (transactions/trash/cas_buffer/sandbox/write_file и т.д.) — это норма.

- [ ] **Step 4: Тесты**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Коммит**

```bash
git add src/daemon/tools.rs
git commit -m "refactor(tools): unregister mutation and sandbox tools from dispatch"
```

---

## Task 2: Снять с поверхности LSP + язык-сервисы

**Files:**
- Modify: `src/daemon/tools.rs` (`dispatch()` ~104-121, `core_tools_list()`)
- Modify: `src/ipc/server.rs` (`handle_tools_list` 615-632, `handle_tools_call` 679-700)

**Interfaces:**
- Produces: `tools/list` отдаёт только read-only ядро (~47 инструментов).
  Вызовы `lsp_*`, `lang_tools_*` и всех `rust_*/ts_*/vue_*/py_*/go_*` →
  `MethodNotFound`.

- [ ] **Step 1: Удалить ветки `lsp_*` и `lang_tools_*` в `dispatch()`**

В `src/daemon/tools.rs` удалить все 16 веток:

```rust
"lsp_goto_definition" => lsp::tool_lsp_goto_definition(args, ctx).await,
"lsp_find_references" => lsp::tool_lsp_find_references(args, ctx).await,
"lsp_hover" => lsp::tool_lsp_hover(args, ctx).await,
"lsp_diagnostics" => lsp::tool_lsp_diagnostics(args, ctx).await,
"lsp_completions" => lsp::tool_lsp_completions(args, ctx).await,
"lsp_inlay_hints" => lsp::tool_lsp_inlay_hints(args, ctx).await,
"lsp_code_actions" => lsp::tool_lsp_code_actions(args, ctx).await,
"lsp_document_symbols" => lsp::tool_lsp_document_symbols(args, ctx).await,
"lsp_workspace_symbols" => lsp::tool_lsp_workspace_symbols(args, ctx).await,
"lsp_goto_implementation" => lsp::tool_lsp_goto_implementation(args, ctx).await,
"lsp_rename" => lsp::tool_lsp_rename(args, ctx).await,
"lsp_expand_macro" => lsp::tool_lsp_expand_macro(args, ctx).await,
"lsp_incoming_calls" => lsp::tool_lsp_incoming_calls(args, ctx).await,
"lsp_outgoing_calls" => lsp::tool_lsp_outgoing_calls(args, ctx).await,
"lang_tools_list" => lang_tools::tool_lang_tools_list(args, ctx).await,
"lang_tools_call" => lang_tools::tool_lang_tools_call(args, ctx).await,
```

- [ ] **Step 2: Удалить схемы `lsp_*`/`lang_tools_*` в `core_tools_list()`**

Удалить `json!({...})` объекты для каждого из 16 имён выше (искать по
`"name": "lsp_goto_definition"` и т.д.).

- [ ] **Step 3: Убрать мёрдж язык-сервисных инструментов в `handle_tools_list`**

В `src/ipc/server.rs` функция `handle_tools_list` (строки ~612-630). Заменить блок:

```rust
    // Try to load project for language-specific tools
    let project_path = req.project_path();

    let mut tool_list = tools::core_tools_list();

    // Append language-specific tools if we have a project context
    if let Some(pp) = project_path {
        if let Ok(project) = state.get_or_load_project(pp).await {
            for svc in project.language_services.iter() {
                for def in svc.tools() {
                    tool_list.push(json!({
                        "name": def.name,
                        "description": def.description,
                        "inputSchema": def.input_schema,
                    }));
                }
            }
        }
    }

    DaemonResponse::success(id, json!({ "tools": tool_list }))
```

на:

```rust
    let tool_list = tools::core_tools_list();
    DaemonResponse::success(id, json!({ "tools": tool_list }))
```

(параметры `req`, `state` остаются в сигнатуре — они используются другими ветками
роутинга; если компилятор укажет на неиспользуемый параметр в этой конкретной
функции, добавить префикс `_`.)

- [ ] **Step 4: Убрать диспатч язык-сервисов в `handle_tools_call`**

В `src/ipc/server.rs` функция `handle_tools_call` удалить блок «Try language
services first» (строки ~679-700, цикл `for svc in project.language_services.iter()`
с `svc.call_tool(...)`), оставив только последующий вызов `tools::dispatch(name, ...)`.
Проверить по коду: после удалённого цикла должен идти путь, вызывающий
`tools::dispatch(name, args, &ctx)`.

- [ ] **Step 5: Сборка**

Run: `cargo build 2>&1 | tail -20`
Expected: SUCCESS (warnings про неиспользуемые `lsp`/`lang_tools`/`languages` норма).

- [ ] **Step 6: Тесты**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: MCP smoke (ручной)**

Пересобрать/переустановить бинарь (`cargo install --path . --force` или как в проекте),
перезапустить демон. Через MCP-клиент проверить:
- `tools/list` → ~47 инструментов, нет `write_file`/`lsp_hover`/`ts_inspect_type`/`run_test`.
- `search` и `get_callers` работают.

> Если ручной smoke недоступен в текущей сессии — отметить шаг как отложенный и
> провести его на задаче 7.

- [ ] **Step 8: Коммит**

```bash
git add src/daemon/tools.rs src/ipc/server.rs
git commit -m "refactor: unregister LSP and language-service tools from MCP surface"
```

---

## Task 3: Удалить файлы хендлеров (мутации, sandbox, LSP, lang_tools)

**Files:**
- Delete: `src/daemon/handlers/transactions.rs`, `trash.rs`, `cas_buffer.rs`,
  `sandbox.rs`, `lsp.rs`, `lang_tools.rs`
- Modify: `src/daemon/handlers/mod.rs`

**Interfaces:**
- Consumes: ничего из удаляемых файлов больше не вызывается (Task 1, 2).
- Produces: модули `transactions/trash/cas_buffer/sandbox/lsp/lang_tools` исчезают.

- [ ] **Step 1: Удалить объявления модулей в `mod.rs`**

В `src/daemon/handlers/mod.rs` удалить строки:

```rust
pub mod cas_buffer;
pub mod lang_tools;
pub mod lsp;
pub mod sandbox;
pub mod transactions;
pub mod trash;
```

- [ ] **Step 2: Удалить файлы**

```bash
git rm src/daemon/handlers/transactions.rs \
       src/daemon/handlers/trash.rs \
       src/daemon/handlers/cas_buffer.rs \
       src/daemon/handlers/sandbox.rs \
       src/daemon/handlers/lsp.rs \
       src/daemon/handlers/lang_tools.rs
```

- [ ] **Step 3: Сборка**

Run: `cargo build 2>&1 | tail -30`
Expected: SUCCESS. Если ошибки — это ссылки из `common.rs`/`state.rs` на
`get_lsp_client`/язык-сервисы; они снимаются в Task 4. На этом этапе ошибок быть
не должно, т.к. `common.rs::get_lsp_client` зависит от `languages/generic_lsp`
(ещё существует), а хендлеры `lsp.rs`/`lang_tools.rs` больше никем не вызываются.
При ошибке вида «cannot find module» — перепроверить, что в `mod.rs` не осталось
ссылок.

- [ ] **Step 4: Тесты**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Коммит**

```bash
git add src/daemon/handlers/mod.rs
git commit -m "refactor: delete mutation, sandbox, and LSP handler modules"
```

---

## Task 4: Удалить языковой слой и его сцепления (state, ToolContext)

**Files:**
- Delete: `src/languages/` (каталог целиком)
- Modify: `src/lib.rs` или `src/main.rs` (объявление `mod languages;`)
- Modify: `src/daemon/state.rs` (поля `language_services`, `lsp_clients`, `init_language_services`)
- Modify: `src/daemon/handlers/common.rs` (`ToolContext`: поля + `get_lsp_client`)
- Modify: `src/ipc/server.rs` (3 места сборки `ToolContext`)

**Interfaces:**
- Produces: `ToolContext` без полей `lsp_clients`/`language_services` и без метода
  `get_lsp_client`. `ProjectState` без этих полей.

- [ ] **Step 1: Найти и удалить объявление модуля `languages`**

```bash
grep -rn "mod languages" src/lib.rs src/main.rs
```

Удалить строку `pub mod languages;` (или `mod languages;`) в найденном файле.

- [ ] **Step 2: Удалить каталог `languages/`**

```bash
git rm -r src/languages/
```

- [ ] **Step 3: Снять `ToolContext` с языкового слоя в `common.rs`**

В `src/daemon/handlers/common.rs`:
- Удалить поле `pub lsp_clients: Arc<RwLock<...GenericLspClient>>>` (строка ~22).
- Удалить поле `pub language_services: Arc<Vec<Box<dyn LanguageService>>>` (строка ~24).
- Удалить метод `pub async fn get_lsp_client(...)` целиком (строки ~31-157).
- Удалить теперь неиспользуемые `use`-импорты `GenericLspClient`, `LanguageService`,
  `RwLock`/`HashMap` (если они больше нигде в файле не нужны — компилятор подскажет).

- [ ] **Step 4: Снять `ProjectState` с языкового слоя в `state.rs`**

В `src/daemon/state.rs`:
- Удалить поле `language_services` (строка ~223) и его инициализацию (строка ~417,
  `let language_services = Arc::new(init_language_services(...));` и присвоение в
  структуру).
- Удалить поле `lsp_clients` (строка ~231) и его инициализацию (строка ~436,
  `lsp_clients: Arc::new(RwLock::new(HashMap::new())),`).
- Удалить функцию `init_language_services()` целиком (строки ~815-844).
- Удалить неиспользуемые импорты (`LanguageService`, `RustService` и т.п.).

- [ ] **Step 5: Починить 3 места сборки `ToolContext` в `ipc/server.rs`**

В `src/ipc/server.rs` в каждом из трёх мест сборки `tools::ToolContext { ... }`
(поля `lsp_clients` на строках 674, 813, 969 — блок сборки начинается на
несколько строк выше) удалить две строки:

```rust
        lsp_clients: Arc::clone(&project.lsp_clients),
        language_services: Arc::clone(&project.language_services),
```

- [ ] **Step 6: Сборка**

Run: `cargo build 2>&1 | tail -40`
Expected: SUCCESS. Компилятор укажет на любые оставшиеся ссылки на удалённые
поля/типы — устранить их (все они в перечисленных выше файлах).

- [ ] **Step 7: Тесты**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 8: Коммит**

```bash
git add -A
git commit -m "refactor: remove language-services layer (src/languages, state, ToolContext)"
```

---

## Task 5: Вырезать частичные функции (file_ops, code_quality, project)

**Files:**
- Modify: `src/daemon/handlers/file_ops.rs` (удалить 5 write-функций)
- Modify: `src/daemon/handlers/code_quality.rs` (удалить format_file, apply_lint_fix)
- Modify: `src/daemon/handlers/project.rs` (удалить add_rule, mark_golden_sample)

**Interfaces:**
- Consumes: эти функции больше не вызываются из `dispatch()` (Task 1).
- Produces: `file_ops.rs` содержит только `tool_list_directory`,
  `tool_get_file_metadata` (+ их helpers); `code_quality.rs` — только `tool_lint_file`;
  `project.rs` — без `tool_add_rule`/`tool_mark_golden_sample`.

- [ ] **Step 1: Удалить write-функции в `file_ops.rs`**

В `src/daemon/handlers/file_ops.rs` удалить функции и их приватные helpers,
используемые только ими: `tool_patch_file`, `tool_write_file`,
`tool_append_to_file`, `tool_create_directory`, `tool_move_file`.
Оставить `tool_list_directory`, `tool_get_file_metadata`. Удалить осиротевшие
импорты/helpers по подсказкам компилятора.

- [ ] **Step 2: Удалить format/fix в `code_quality.rs`**

В `src/daemon/handlers/code_quality.rs` удалить `tool_format_file` и
`tool_apply_lint_fix` (+ их приватные helpers). Оставить `tool_lint_file`.

- [ ] **Step 3: Удалить tuning-функции в `project.rs`**

В `src/daemon/handlers/project.rs` удалить `tool_add_rule` и
`tool_mark_golden_sample`. Оставить `tool_get_dependencies`,
`tool_dependency_impact`, `tool_project_tree`, `tool_domain_stats`,
`tool_get_vue_tree`.

- [ ] **Step 4: Сборка**

Run: `cargo build 2>&1 | tail -30`
Expected: SUCCESS.

- [ ] **Step 5: Тесты**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Коммит**

```bash
git add src/daemon/handlers/file_ops.rs src/daemon/handlers/code_quality.rs src/daemon/handlers/project.rs
git commit -m "refactor: drop write/format/tuning functions from handlers"
```

---

## Task 6: Зависимости Cargo.toml + финальная зачистка мёртвого кода

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: сборка без предупреждений о мёртвом коде удалённых подсистем.

- [ ] **Step 1: Убрать LSP-зависимости**

В `Cargo.toml` удалить:

```toml
lsp-types = "0.97"
```

И, если после задач 1–5 они больше не используются (проверить grep), удалить
`shell_words` и `which`:

```bash
grep -rn "shell_words\|which::\|::which" src/
```

Если вывод пуст — удалить обе строки из `Cargo.toml`. `reqwest` НЕ трогать
(используется `indexer/embedder.rs` и `parser/lang_manager`).

- [ ] **Step 2: Сборка с проверкой предупреждений**

Run: `cargo build 2>&1 | tail -40`
Expected: SUCCESS. Просмотреть warnings: устранить оставшийся мёртвый код
(неиспользуемые импорты, приватные функции, поля) по подсказкам компилятора.
Цель — ноль warnings про удалённые подсистемы.

- [ ] **Step 3: Clippy (если используется в проекте)**

Run: `cargo clippy 2>&1 | tail -40`
Expected: без новых ошибок. Устранить тривиальные замечания по затронутым файлам.

- [ ] **Step 4: Тесты**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Коммит**

```bash
git add -A
git commit -m "chore: drop unused deps (lsp-types, shell_words, which) and dead code"
```

---

## Task 7: Документация, описание сервера, память; финальный smoke

**Files:**
- Modify: `README.md`, `docs/desc/OVERVIEW.md`, `.mcp.json`
- Modify: `/home/e5ash/.claude/projects/-home-e5ash-storage-vibe-gofer/memory/project_gofer.md`

- [ ] **Step 1: Обновить README и OVERVIEW**

Переписать позиционирование на «read-only язык-агностичный index-search /
navigation MCP». Убрать упоминания LSP-инструментов, рефакторинга, sandbox,
транзакций, clipboard. Описать актуальный набор (~47 read-only инструментов).

- [ ] **Step 2: Обновить `.mcp.json`**

Если в `.mcp.json` есть текстовое описание сервера/инструментов — привести к
новому позиционированию.

- [ ] **Step 3: Обновить память проекта**

В `project_gofer.md` отразить: gofer теперь read-only поисковик; удалены
languages/, LSP, мутации, sandbox, транзакции, trash, CAS; ~47 инструментов;
ноль внешних процессов кроме эмбеддера.

- [ ] **Step 4: Финальный MCP smoke**

Пересобрать/переустановить бинарь, перезапустить демон. Проверить:
- `tools/list` → ~47 инструментов; отсутствуют `write_file`, `lsp_*`, `ts_*`,
  `vue_*`, `py_*`, `go_*`, `rust_*`, `run_test`, `clipboard_*`, `*_transaction`.
- `search`, `skeleton`, `get_callers`, `get_references`, `read_function_context`
  работают.
- Индексация проекта (любой язык) проходит через tree-sitter (язык-агностичный
  парсинг не затронут).

- [ ] **Step 5: Коммит**

```bash
git add README.md docs/desc/OVERVIEW.md .mcp.json
git commit -m "docs: reposition gofer as read-only index-search MCP"
```

---

## Task 8 (опционально): Миграция drop неиспользуемых таблиц

> Решение отложено в спеке. Делать только если хотим косметически зачистить схему.
> Неиспользуемые таблицы безвредны; задачу можно пропустить.

**Files:**
- Create: `src/storage/migrations/018_drop_unused.sql`

- [ ] **Step 1: Определить кандидатов**

Просмотреть миграции 001–017 и найти таблицы, относящиеся ТОЛЬКО к удалённым
подсистемам (кандидаты: trash, transactions, cas_buffer/clipboard, golden_samples,
audit_log). Подтвердить grep'ом, что эти таблицы больше нигде в `src/` не читаются.

- [ ] **Step 2: Написать миграцию**

```sql
-- 018_drop_unused.sql — drop tables of removed mutation/LSP subsystems
DROP TABLE IF EXISTS golden_samples;
DROP TABLE IF EXISTS audit_log;
-- + любые подтверждённые таблицы trash/transactions/cas_buffer
```

(Точный список таблиц подставить по результатам Step 1 — указывать только
подтверждённые имена.)

- [ ] **Step 3: Сборка + тесты (миграции применяются при инициализации БД)**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS (миграция применяется на свежей БД без ошибок).

- [ ] **Step 4: Коммит**

```bash
git add src/storage/migrations/018_drop_unused.sql
git commit -m "chore(db): drop tables of removed subsystems"
```

---

## Итоговая верификация (после всех задач)

- `cargo build` — зелёный, без warnings о мёртвом коде удалённых подсистем.
- `cargo test` — все тесты проходят.
- `tools/list` через MCP — ~47 read-only инструментов.
- Удалено ~76 инструментов и ~11K строк (~31% базы).
- Нативная навигация/поиск и индексация работают как прежде.
