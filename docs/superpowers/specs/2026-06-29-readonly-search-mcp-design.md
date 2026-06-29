# gofer → read-only code-search MCP

**Дата:** 2026-06-29
**Статус:** утверждён, ожидает плана реализации
**Ветка:** feat/code-intel-tools

## Цель

Превратить gofer из MCP-сервера с инструментами поиска **и** рефакторинга в
**чистый read-only поисковик/навигатор по коду**. Убрать все инструменты,
которые мутируют код пользователя или выполняют произвольный код.

### Мотивация

1. **Фокус/позиционирование** — один чёткий продукт: лучший MCP для поиска и
   навигации по кодовой базе, без размывания на рефакторинг.
2. **Уход от дублирования** — мутации (`write_file`, `patch_file`, `move_file`,
   clipboard) дублируют нативные инструменты хост-агента (Claude Code/Cursor),
   которые отлажены и пользователю привычны.
3. **Безопасность/доверие** — read-only сервер проще доверять: нет риска порчи
   файлов, не нужны trash/transactions/откаты.
4. **Меньше кода/поддержки** — удаление мутаций, sandbox, CAS-буфера и
   транзакций сокращает ~3K+ строк хендлеров и связанную инфраструктуру.

## Объём

**87 → ~62 инструмента.** Удаляются 25 инструментов (мутации кода + выполнение
кода). Архитектура уже разделяет read-only и write-логику, поэтому риск
регрессии умеренный, циклических зависимостей между категориями нет.

### Удаляется (25 инструментов)

**Файлы хендлеров целиком:**

| Файл | Инструменты |
|---|---|
| `src/daemon/handlers/transactions.rs` (622) | begin_transaction, add_operation, commit_transaction, rollback_transaction, list_transactions |
| `src/daemon/handlers/trash.rs` (362) | delete_safe, list_trash, restore, purge_trash |
| `src/daemon/handlers/cas_buffer.rs` (525) | clipboard_copy, clipboard_paste, clipboard_replace, clipboard_store_text, clipboard_list, clipboard_clear |
| `src/daemon/handlers/sandbox.rs` (557) | execute_code, execute_function, run_test, run_all_tests |

**Частичная правка файлов (удалить только перечисленные функции):**

| Файл | Удалить | Оставить |
|---|---|---|
| `file_ops.rs` (469) | write_file, patch_file, append_to_file, create_directory, move_file | list_directory, get_file_metadata |
| `code_quality.rs` (705) | format_file, apply_lint_fix | lint_file |
| `project.rs` (235) | add_rule, mark_golden_sample | get_dependencies, dependency_impact, project_tree, domain_stats, get_vue_tree |
| `lsp.rs` (785) | lsp_rename | все остальные lsp_* (read-only) |

### Остаётся (read-only ядро, ~62 инструмента)

- **Поиск:** search, search_by_purpose, smart_file_selection, structural_search
- **Символы/граф вызовов:** get_symbols, get_references, get_callers, get_callees,
  call_path, dependency_subgraph, find_implementations, find_unused_symbols,
  find_by_type_signature, search_symbols, symbol_exists, is_exported, и т.д.
- **Чтение файлов:** read_file, skeleton, read_function_context, read_types_only,
  context_bundle, find_files, grep, find_unused_imports, file_exists
- **Git (read-only):** git_blame, git_history, git_diff, suggest_commit, verify_patch
- **Проект:** project_tree, get_dependencies, dependency_impact, domain_stats, get_vue_tree
- **Анализ качества (read-only):** complexity, find_unreachable, lint_file
- **Диагностика/индекс:** run_diagnostics, health_check, get_config_keys,
  has_tests_for, get_index_status, validate_index, get_cache_stats,
  get_query_stats, **force_reindex** (операционное управление индексом)
- **Батч:** batch_operations — уже read-only (батчит только read_file,
  get_symbols, search, skeleton, get_references, read_function_context,
  read_types_only). Без изменений.
- **LSP (read-only):** lsp_goto_definition, lsp_find_references, lsp_hover,
  lsp_diagnostics, lsp_completions, lsp_inlay_hints, lsp_document_symbols,
  lsp_workspace_symbols, lsp_goto_implementation, lsp_expand_macro,
  lsp_incoming_calls, lsp_outgoing_calls, lsp_code_actions (см. открытый вопрос)
- **Мета:** lang_tools_list, lang_tools_call

### Решения по пограничным зонам

- **Sandbox/выполнение кода** — удалить целиком. Строго read-only, никакого
  выполнения произвольного кода или тестов.
- **CAS-буфер (clipboard)** — удалить целиком; это помощник переноса кода
  (по сути рефакторинг).
- **add_rule / mark_golden_sample** — удалить (тюнинг качества поиска через
  запись в БД gofer).
- **force_reindex** — оставить: «read-only по коду пользователя», но ручное
  управление индексом нужно операционно (watcher авто-индексирует, но ручной
  триггер полезен).
- **verify_patch** — оставить: валидация патча без применения (read-only);
  хост-агент может проверять свои патчи.

## Подход: поэтапное удаление (вариант C)

Тот же конечный результат, что и жёсткое удаление, но по шагам с проверкой —
на каждом шаге `cargo build` зелёный.

### Фаза 1 — снять с поверхности MCP

В `src/daemon/tools.rs`:
- Удалить 25 веток `match` в `dispatch()`.
- Удалить соответствующие `json!()` блоки в `core_tools_list()`.

Результат: инструменты исчезают из `tools/list`, вызов даёт `MethodNotFound`.
Код хендлеров ещё на месте (будет помечен компилятором как неиспользуемый).

### Фаза 2 — удалить код

- В `src/daemon/handlers/mod.rs` убрать `pub mod transactions; pub mod trash;
  pub mod cas_buffer; pub mod sandbox;`.
- Удалить файлы: `transactions.rs`, `trash.rs`, `cas_buffer.rs`, `sandbox.rs`.
- Вырезать частичные функции из `file_ops.rs`, `code_quality.rs`, `project.rs`,
  `lsp.rs`.
- Зачистить мёртвый код по предупреждениям компилятора: неиспользуемые helpers
  в `common.rs`, импорты, вспомогательные структуры.

### Фаза 3 — зачистка

- Обновить документацию: `README`, `docs/desc/OVERVIEW.md`, описание сервера в
  `.mcp.json` → позиционирование «read-only code-search/navigation MCP».
- Обновить память проекта (`project_gofer.md`).
- **Опционально:** миграция `018_drop_unused.sql` — drop таблиц trash,
  transactions, cas_buffer, golden_samples, audit_log (если относятся только к
  удалённым подсистемам). Миграции append-only — существующие 001–017 не трогаем;
  неиспользуемые таблицы безвредны, drop — косметика.

## Зависимости и состояние

- Ссылки на удаляемые подсистемы сосредоточены в `tools.rs` (dispatch + схемы) и
  в самих файлах хендлеров.
- `ToolContext` (`common.rs`) **не** держит менеджеров транзакций/корзины/
  буфера/sandbox в долгоживущем состоянии — модули самодостаточны (используют
  ctx для project root / БД). Подтверждено grep'ом: в `daemon/*.rs` и `common.rs`
  нет инициализации этих менеджеров вне самих хендлеров.
- Это нужно перепроверить при реализации: если в state/registry всё же есть
  инициализация (например, создание trash-директории или CAS-стора при старте) —
  убрать её на Фазе 2.

## Тестирование и верификация

После **каждой** фазы:
- `cargo build` — зелёный (Фаза 1: с предупреждениями о неиспользуемом коде, что
  нормально; Фазы 2–3: без предупреждений о мёртвом коде удалённых модулей).
- `cargo test` — существующие `#[cfg(test)]` тесты проходят.

MCP smoke-тест (после Фазы 1 и финально):
- `tools/list` отдаёт ~62 инструмента (не 87).
- Вызов удалённого инструмента (напр. `write_file`) → `MethodNotFound`.
- `search`, `skeleton`, `get_callers`, `read_function_context` работают как раньше.

## Открытые вопросы (решить при реализации)

1. **`lsp_code_actions`** — возвращает список доступных действий (read-only) или
   применяет их? По умолчанию считаем read-only листингом (фактическое
   применение шло через `lsp_rename`, который удаляем). Проверить в `lsp.rs`;
   если применяет — удалить.
2. **Состояние демона** — есть ли инициализация trash-директории / CAS-стора /
   sandbox-раннера при старте демона (state.rs/registry). Если да — убрать.
3. **Миграция `018_drop_unused.sql`** — делать ли drop неиспользуемых таблиц или
   оставить их как безвредный артефакт. Решение можно отложить до конца Фазы 3.
