# Аудит инструментов и техническая roadmap

Срез по состоянию **2026-05-28**. Цель документа — оценить целесообразность существующего набора MCP-инструментов и зафиксировать критические пробелы для **технической** работы агента (поиск, навигация, дебаг). Инструменты планирования, knowledge management и продуктовые обвязки сюда **не входят**.

Привязки к коду точечные — если рефакторили `src/`, проверь актуальность.

> **Прогресс реализации (2026-05-28):**
>
> Блок A (чистка):
> - `search_files` удалён, логика инлайнена в `grep` (+ `context_lines` параметр).
> - `run_check` удалён.
> - `has_documentation` удалён.
> - `run_diagnostics` получил параметр `file` для фильтрации.
> - Транзакции (`begin_transaction`/`add_operation`/`commit_transaction`/`rollback_transaction`/`list_transactions`) зарегистрированы в dispatch с полными JSON-Schema.
>
> Блок B (P0 фичи):
> - **`find_unused_symbols`** — символы без incoming ссылок. Фильтры: `kind`, `file`, `public_only`, `exclude_tests`, `exclude_entry_points`, `exclude_exports`, `limit`. SQL поверх `symbol_references` с фильтром на `kind IN ('call', 'usage', 'inherit', 'type_usage')`.
> - **Расширение парсера**: для Rust атрибуты (`#[test]`, `#[no_mangle]`, …) теперь попадают в `signature` через walk по prev_sibling `attribute_item` (`parser/core.rs`). Без этого attribute-based фильтрация мёртвого кода не работала на Rust. **Требует `force_reindex` старых проектов.**
> - **Расширенные heuristics**: тесты — по пути + по атрибутам в signature + по naming-конвенции (`test_*`, `*_test`); entry points — `main`/`__main__`/`lambda_handler`/`handler`; FFI/wasm/Python экспорты — `#[no_mangle]`, `extern "C"`, `#[wasm_bindgen]`, `#[pyfunction]`, `#[napi]`, `@customElement`, `@Component`.
> - **`structural_search`** — поиск по AST-форме через tree-sitter queries. Каталог из 12 пресетов для Rust/TS/Python (unwrap, panic, todo, dbg, println, clone, expect, any, console.log, @ts-ignore, print, bare except). Custom S-expression через `query` + `language`. Без аргументов отдаёт каталог. Возвращает hits с file:line:col + матчевый текст.
> - **`call_path`** — BFS поверх `symbol_references` от `from` до `to` через outgoing (`calls`) или incoming (`called_by`) edges. Возвращает shortest paths с nodes/edges/файлами. Резолвит unresolved refs через имя (cap 16 кандидатов).
> - **`find_unused_imports`** — импорты файла, локальное имя которых нигде не используется. Расширен парсер `collect_*_imports` (для Rust/Python/Go) — теперь `ImportInfo.items` содержит локальные имена (ранее было пусто для всех языков кроме TS). Покрыто 21 unit-тестом на парсеры имён.
> - **`complexity`** — цикломатическая сложность + size-метрики (lines/params/nesting) на функцию. tree-sitter обход с подсчётом decision points. Union node-kinds для Rust/TS/Python/Go + операторы `&&`/`||`/`??`. Вложенные функции — отдельные записи, замыкания — в счёт обрамляющей. Покрыто 9 unit-тестами.
> - **`find_unreachable`** — статически недостижимый код после терминаторов (return/break/continue/throw/raise/panic-макросы) в том же блоке. Только прямые сиблинги → near-zero false positives. Покрыто 5 unit-тестами (включая защиту от false positive на `return` внутри `if`).
> - **`find_by_type_signature`** — region-aware поиск по return-типу / типу параметра / подстроке сигнатуры. `split_signature` — чистая строковая функция, покрыта 10 тестами на литералах Rust/TS/Python/Go.
> - **Кросс-язычность закрыта:** decorator-детекция (Python/TS) в `parser/core.rs`; Go-пресеты + расширенный каталог `structural_search` + compile-тест (поймал битый `ts_any`).
> - **`dependency_subgraph`** — BFS-окрестность символа (out/in/both), nodes+edges, для impact-анализа.
> - **`find_implementations`** — кросс-язычный поиск реализаций (Rust impl / TS implements,extends / Python базы). Whole-token матчинг. Покрыто 8 тестами. Все 116 тестов зелёные.

## Содержание

1. [Лишнее и сомнительное](#1-лишнее-и-сомнительное)
2. [Что укрупнить](#2-что-укрупнить)
3. [Чего критически не хватает — поиск](#3-чего-критически-не-хватает--поиск)
4. [Чего критически не хватает — дебаг и runtime](#4-чего-критически-не-хватает--дебаг-и-runtime)
5. [Что добавить параметрами в существующие](#5-что-добавить-параметрами-в-существующие)
6. [Приоритизация](#6-приоритизация)

## 1. Лишнее и сомнительное

### 1.1 Чистые дубли

| Инструмент | Дублирует | Доказательство в коде | Решение | Статус |
|---|---|---|---|---|
| `search_files` | `grep` | `tool_grep` был forwarder'ом, переименовывал параметры и вызывал `tool_search_files`. Уникальной логики не имел. | Удалить `search_files`, оставить `grep`. | ✅ сделано |
| `run_check` | `run_diagnostics` | `diagnostics.rs:86–89` — `tool_run_check` буквально `tool_run_diagnostics(args, ctx).await`. | Удалить `run_check`. | ✅ сделано |

### 1.2 Заглушки и низкое качество

| Инструмент | Состояние |
|---|---|
| ~~`has_documentation`~~ | ✅ **Удалён** — возвращал всегда `has_docs: false`. Если когда-то понадобится — реализовать через распарсинг doc-комментов в индексе с флагом на `Symbol`, а не заглушкой. |
| `has_tests_for` (`diagnostics.rs:190+`) | Path-matching по шаблонам (`*.test.ts`, `_test.rs`, …). Не использует индекс. Заменить на `find_tests_for(symbol)` через поиск тестов, импортирующих/вызывающих символ. |
| Транзакции (`begin_transaction`, `add_operation`, `commit_transaction`, `rollback_transaction`, `list_transactions`) | ✅ **Зарегистрированы в dispatch + JSON-Schema.** Handlers в `handlers/transactions.rs` со снапшотами и автооткатом. |

### 1.3 Вне технического скоупа

Эти не про поиск/дебаг, а про процесс/knowledge. Если рамка — «технический copilot», их надо вынести в отдельный namespace (по аналогии с `lang_tools_*`), чтобы не засорять основной `tools/list`:

- `add_rule`, `mark_golden_sample` — knowledge base, проектные конвенции.
- `domain_stats` — аналитика структуры.
- `get_config_keys` — реестр конфиг-ключей.
- `suggest_commit` — генерация commit-message.
- `get_vue_tree` — нишевая Vue-фича.

Сами по себе они не плохи, но в основном listе кушают ~600 токенов схем + добавляют шум в выборе агенту.

### 1.4 Размытые границы между похожими

| Триплет | Как сейчас | Что правильнее |
|---|---|---|
| `get_symbols` / `search_symbols` | Первый — пагинированный из rkyv-кеша (`symbols.rs:7–120`), второй — FTS по имени без пагинации (`symbols.rs:140–183`). | Слить в `find_symbols(file?, query?, kind?, offset, limit)` с двумя путями внутри: query=empty → пагинированный, query=set → FTS. |
| `get_references` / `get_callers` / `lsp_find_references` | Первые два — на SQLite (`symbols.rs:122–138, 185–207`), причём `get_callers` — строгий подмножественный фильтр `ref_kind ∈ {call, usage}` от `get_references`. Третий — LSP, требует line/character. | Оставить **два** уровня: `references(symbol, kind?)` поверх SQLite (фильтр кинда — параметр) и `lsp_find_references` для точных позиций. |
| `run_diagnostics` / `lsp_diagnostics` / `check_code` (после слияния) | Три источника диагностики: cargo/tsc один-раз, live LSP, lint. | Этот разрыв обоснован — оставить, но в описаниях явно проговорить «когда какой». Сейчас агент гадает. |

### 1.5 Дубли, обусловленные архитектурой (не трогать)

`rust_goto_definition`, `rust_find_references`, `rust_hover`, `rust_diagnostics`, `rust_completions`, `rust_inlay_hints`, `rust_code_actions` (`languages/rust.rs:265–302`) — буквально вызывают одноимённые `lsp_*` и сериализуют через `to_string_pretty`. Это **не баг**: они доступны через `lang_tools_call`, экономят место в основном `tools/list`. Оставить.

## 2. Что укрупнить

Несколько мелких инструментов с похожим скоупом можно слить — это сэкономит ~500–800 токенов схем в каждом контексте.

### 2.1 Корзина → один инструмент

Сейчас: `delete_safe`, `restore`, `list_trash`, `purge_trash`.

```
trash(action: "delete"|"restore"|"list"|"purge", ...)
```

### 2.2 CAS-буфер → 3 инструмента вместо 6

Сейчас: `clipboard_copy`, `clipboard_paste`, `clipboard_replace`, `clipboard_store_text`, `clipboard_list`, `clipboard_clear`.

Предлагается:
```
clip_capture(source: "range"|"content", ...)    # copy + store_text
clip_apply(target: "insert"|"replace", ...)     # paste + replace
clip_manage(action: "list"|"clear", ...)
```

### 2.3 Диагностики

После слияния `run_check` в `run_diagnostics`:
```
check_code(workspace?, all_targets?, package?, manifest_path?, file?)
```
И параметр `file` (которого сейчас нет) сразу закроет частый кейс «дай диагностики только по этому файлу».

### 2.4 `append_to_file` — оставить

Изначально я думал, что это сводимо к `patch_file` с пустым search. Это **не так**: `file_ops.rs:435–489` использует `OpenOptions::append(true)` — атомарный append на уровне fs без чтения файла, плюс семантика `newline_before`. Дешевле и безопаснее для логов, переменных окружения, добавления строк в конец. **Оставить.**

## 3. Чего критически не хватает — поиск

Это главный пробел. Сейчас агент при сложных задачах валится в regex по `grep`, в результат прилетают комментарии, строки и тесты вперемешку с реальным кодом.

### 3.1 Структурный поиск (P0, частично сделано)

Поверх tree-sitter, который уже подключён и парсит каждый файл.

| Инструмент | Статус | Что делает |
|---|---|---|
| `structural_search` | ✅ реализован | Поиск по AST-патерну (tree-sitter query). Каталог 12 пресетов для Rust/TS/Python + custom S-expression через `query` + `language`. |
| `find_pattern_violations` | 🟡 покрывается пресетами | Часть каталога вкомпилена в `structural_search` (`rust_unwrap`, `rust_dbg`, `ts_any`, `py_bare_except`, …). Дальнейшие паттерны добавлять туда же. |

### 3.2 Типо-ориентированный поиск (P0, частично сделано)

| Инструмент | Статус | Что делает |
|---|---|---|
| `find_by_type_signature` | ✅ реализован | Region-aware поиск по return/param-типу. «Функции, возвращающие `Result<MyType, _>`», «методы с `&mut Connection`». Покрыто 10 unit-тестами (Rust/TS/Python/Go литералы + generics edge case). |
| `find_implementations` | ✅ реализован | Кросс-язычно: Rust `impl T for`, TS `implements`/`extends`, Python базовые классы. Go вне скоупа (структурный). Покрыто 8 unit-тестами. |
| `find_overrides` | 🟡 покрывается | `find_implementations` с `extends` частично закрывает (кто наследует базу). Точного override-метода нет. |

В SQLite уже есть `symbols.signature` — хороший fallback по подстроке + AST-уточнение через tree-sitter для точности.

### 3.3 Транзитивный поиск (P1, частично сделано)

| Инструмент | Статус | Что делает |
|---|---|---|
| `call_path` | ✅ реализован | BFS shortest path с поддержкой `calls`/`called_by` направлений. |
| `dependency_subgraph` | ✅ реализован | BFS-окрестность с `out`/`in`/`both`, nodes+edges, bounded depth/nodes. |
| `reachability(from, to)` | 🟡 покрывается | `call_path` отвечает на «достижим ли» — если `paths` не пуст, достижим. |

Граф уже есть в `symbol_references`.

### 3.4 Similarity (P2)

| Инструмент | Что делает |
|---|---|
| `find_similar_code` | По эмбеддингу куска кода (на вход — текст, не запрос). Дедупликация, rewriting candidates. |
| `find_duplicates` | Скан проекта на структурные дубли (нормализованный AST hash). |

`LanceStorage::search` уже умеет принимать произвольный `query_vector` — достаточно эмбеддить вход и вызвать.

### 3.5 История (P2)

| Инструмент | Что делает |
|---|---|
| `git_grep_history` | Поиск по всем коммитам, не текущему состоянию. |
| `when_was_added(symbol)` / `when_was_removed(symbol)` | Когда символ появился/исчез. |
| `blame_range(file, start, end)` | Сводный blame диапазона. У текущего `git_blame` есть `start_line`/`end_line`, но он отдаёт построчно, не агрегирует. |

git2 в проекте уже есть — обвязка прямолинейная.

## 4. Чего критически не хватает — дебаг и runtime

### 4.1 Runtime-наблюдение (P0 — самый зияющий пробел)

Сейчас `execute_code` запускает и отдаёт stdout. Этого мало для дебага.

| Инструмент | Что делает |
|---|---|
| `run_with_trace` | Запустить функцию/тест, получить trace вызовов с временем. Реализация: cargo-flamegraph/py-spy/`node --inspect`. |
| `profile_function` | CPU/memory профиль одного вызова. |
| `capture_panic_backtrace` | Запустить, поймать panic, отдать backtrace с разрешёнными именами (`RUST_BACKTRACE=full`, `python -X faulthandler`). |
| `run_with_strace` | Системные вызовы — для дебага I/O проблем. |
| `instrument_function` | Пропатчить функцию println-логами в ключевых точках, запустить, откатить (поверх существующих транзакций — повод их таки включить). |

### 4.2 Тесты — глубже pass/fail (P0)

| Инструмент | Что делает |
|---|---|
| `run_test_with_capture` | Тест + stdout/stderr/panic/assertion-details. Сейчас `run_test` отдаёт грубый pass/fail. |
| `test_diff(test, ref_a, ref_b)` | Запустить тест на двух коммитах/ветках, показать различия. Когда «вчера работало, сегодня нет». |
| `find_flaky_tests` | N прогонов, статистика failure rate. |
| `bisect_regression(failing_test, last_good, first_bad)` | `git bisect` для падающего теста. |
| `find_related_tests(symbol)` | Тесты, реально покрывающие символ — поверх coverage-данных, а не `has_tests_for`. |

### 4.3 Coverage (P0)

Полностью отсутствует. Без coverage агент не знает, что трогать безопасно, какие ветки не покрыты.

| Инструмент | Что делает |
|---|---|
| `get_coverage(scope)` | Покрытие тестами по строкам/функциям. tarpaulin/llvm-cov для Rust, coverage.py, jest. |
| `uncovered_branches(file)` | Где нет покрытия. |
| `coverage_diff(ref_a, ref_b)` | Покрытие сейчас vs до изменения. |

### 4.4 Компиляторные ошибки — с контекстом (P0)

| Инструмент | Что делает |
|---|---|
| `explain_error_at(file, line)` | Берёт диагностику, добавляет: определения упомянутых типов, доступные impl'ы, релевантные trait'ы, `rustc --explain`. Сейчас агент после «cannot find trait X» сам ищет X. |
| `suggest_fixes(file, line)` | LSP code actions + индексные кандидаты (близкие имена, тривиальные правки). |
| `compile_blame(file, line)` | Что в незакоммиченном diff'е подняло конкретную ошибку — какая строка её источник. |
| `find_similar_errors` | Та же ошибка где-то ещё в проекте. |

### 4.5 Семантика языка (P1)

| Инструмент | Что делает |
|---|---|
| `type_at(file, line, char)` | Какой тип у выражения в позиции. `lsp_hover` отдаёт строку (включая markdown) — нужно структурно. |
| `infer_chain(var)` | Цепочка type-inference для переменной (Rust/TS). |
| `lifetime_info(symbol)` | Rust: lifetimes у функции/типа, где конфликт. |
| `borrow_at(file, line, char)` | Rust: что заимствует переменная в точке. |
| `expand_inline(file, line, char)` | TS/Vue: развернуть тип/computed property inline. |

### 4.6 Мёртвый код (P1, почти закрыто — 3 из 4)

| Инструмент | Статус | Что делает |
|---|---|---|
| `find_unused_symbols` | ✅ реализован | Публичные/приватные функции/типы без callers, обратный обход графа ссылок. |
| `find_unused_imports` | ✅ реализован | Импорты файла, локальное имя которых не используется. Rust/TS/Python/Go. |
| `find_unreachable` | ✅ реализован | Код после `return`/`break`/`continue`/`throw`/`raise`/panic-макросов в том же блоке. Недостижимые match-ветки — пока вне скоупа. |
| `find_dead_features` | 🔵 в планах | Cargo features, которые включены, но не используются ни в одном пути сборки. |

`get_unused_dependencies` уже есть (`storage/sqlite.rs:926`) — переименовать и сгруппировать.

### 4.7 Метрики сложности (P1, существенно сделано)

| Инструмент | Статус | Что делает |
|---|---|---|
| `complexity` | ✅ реализован | Цикломатическая сложность + size-метрики (lines/params/nesting) на функцию. Покрывает `cyclomatic_complexity` и `function_size_metrics`. |
| `cognitive_complexity(file)` | 🔵 в планах | Ближе к восприятию, чем cyclomatic (штраф за вложенность). |
| `hotspot_complexity(top_n)` | 🔵 в планах | Топ функций по cyclomatic × churn (нужна git-churn интеграция). |

## 5. Что добавить параметрами в существующие

Эти дёшевы — закрывают большие пробелы без новых инструментов.

| Инструмент | Новый параметр | Зачем |
|---|---|---|
| `search` | `code_only: bool` | Игнорировать комментарии/строки. Сейчас «authentication» ловит docstring'и. Реализуется фильтром по `chunk.kind` или AST-проходом перед эмбеддингом. |
| `grep` | `in_kind: "function"\|"test"\|"comment"\|...` | AST-фильтр по виду конструкции, не текстовый. |
| `read_file` | `with_diagnostics: bool` | Рядом со строкой — пометки LSP/clippy. Часто для дебага достаточно одного запроса вместо двух. |
| `get_callers` / `get_callees` | `depth: usize` | Транзитивный обход на N уровней. Закроет половину `call_path`. |
| `lsp_diagnostics` | `include_explanation: bool` | Для каждой ошибки тут же `rustc --explain Ennnn` или эквивалент. |
| `run_test` | `verbose: bool` | Промежуточный stdout, не только финальный pass/fail. |
| `skeleton` | `include_calls: bool` | Внутри тел функций оставлять список вызовов как комментарий. Гибрид skeleton+context. |
| `git_blame` | `aggregate: bool` | Сводный blame диапазона (от → до), не построчно. |
| `verify_patch` | `with_diagnostics: bool` (live, не cargo) | Гонять через LSP вместо `cargo check` — быстрее. |
| `find_files` | `min_size`, `max_size`, `modified_after` | Часть кейсов «найди что недавно менялось». |

## 6. Приоритизация

Порядок реализации, если бы я строил roadmap для технического copilot'а.

### P0 — нужно вчера

Без этих инструментов агент **не может нормально дебажить**, регулярно мажет, требует много туда-сюда.

1. ~~**`structural_search`**~~ — ✅ реализован (с каталогом 12 пресетов + custom queries).
2. **`run_with_trace` / `capture_panic_backtrace`** — без них дебаг = чтение принтлогов глазами.
3. **`explain_error_at`** с контекстом типов — главный поток дебага.
4. **`get_coverage`** + `uncovered_branches` — критично для понимания, что трогать.
5. ~~**`find_unused_symbols`**~~ — ✅ реализован.
6. ~~**`find_by_type_signature`**~~ — ✅ реализован (region-aware, 4 языка).

### P1 — заметный апгрейд

7. ~~`call_path`, `dependency_subgraph`~~ — ✅ реализованы.
8. ~~`cyclomatic_complexity` + `function_size_metrics`~~ — ✅ реализованы в `complexity`.
9. `bisect_regression` — отладка регрессий.
10. `run_test_with_capture` — глубже pass/fail.
11. `type_at` / `infer_chain` — семантические запросы.
12. ~~`find_unused_imports`, `find_unreachable`~~ — ✅ реализованы.

### P2 — полировка

13. `find_similar_code` — рефакторинг.
14. `git_grep_history`, `when_was_added` — историческое.
15. `find_flaky_tests` — стабилизация CI.
16. `instrument_function` — print-debug автоматизация (зависит от транзакций).
17. `find_overrides`, `expand_inline`.

### Параллельные чистки

Дёшевы, освобождают токены в основном `tools/list`:

- Удалить `search_files` (дубль `grep`).
- Слить `run_diagnostics` + `run_check` → `check_code(file?)`.
- Удалить `has_documentation` (или починить).
- Слить trash → один `trash(action)`.
- Слить CAS-буфер 6 → 3 (`clip_capture`/`clip_apply`/`clip_manage`).
- Решить транзакции: включать в dispatch или удалять handler.
- Вынести knowledge-инструменты (`add_rule`, `mark_golden_sample`, `domain_stats`, `get_config_keys`, `suggest_commit`, `get_vue_tree`) в отдельный namespace через тот же приём, что `lang_tools_call` — оставить функционал, убрать из общего listа.

Грубо: P0 закрывает ~80% болей, P1 — ещё 15%. P2 — про комфорт.

## 6.5 Покрытие по языкам (honest matrix)

Новые инструменты сессии — где реально работают. ✓ = верифицировано unit-тестом, ⚠️ = по механизму должно работать, не проверено (нет установленной грамматики), 🟡 = частично.

| Инструмент | Rust | Python | TS/JS | Go | Природа |
|---|---|---|---|---|---|
| `call_path` | ✓ | ✓ | ✓ | ✓ | Язык-агностичен — обход `symbol_references`. |
| `find_unused_imports` | ✓ | ✓ | ✓ | ✓ | Парсеры имён + тесты на все 4 языка. |
| `complexity` | ✓ | ✓ | ✓ | ⚠️ | Union node-kinds. Py/TS проверены тестами; Go — kinds в union, грамматика не установлена в этом окружении. |
| `find_unreachable` | ✓ | ✓ | ✓ | ⚠️ | Терминаторы `return`/`break`/`continue` универсальны, `raise`/`throw`/`goto` покрыты. panic-макросы — Rust-only (но `raise`/`throw` закрывают Python/TS). |
| `structural_search` | ✓ 7 | ✓ 3 | ✓ 5 | ⚠️ 2 | Механизм универсален (custom `query`). Каталог: Rust 7, Python 3, TS 5, Go 2. Rust/Py/TS пресеты verified compile+match-тестами; Go пресеты compile-проверяются только при установленной грамматике. |
| `find_unused_symbols` | ✓ | ✓ | ✓ | 🟡 | Граф язык-агностичен. Attribute/decorator-фильтры теперь работают для Rust (`#[test]`), Python (`@pytest.fixture`) и TS (`@Component`) — декораторы попадают в `signature` (см. ниже). Go — без attribute-маркеров, path+naming. |

### Закрытые Rust-перекосы (было TODO, сделано)

1. ✅ **`structural_search`: Go-пресеты добавлены** (`go_panic`, `go_fmt_print`) + расширен Python (`py_breakpoint`) и TS (`ts_debugger`, `ts_non_null`). Добавлен **compile-тест** на все пресеты — он сразу поймал, что `ts_any` всё это время был с битым S-expression (зафикшено).
2. ✅ **`find_unused_symbols`: decorator-детекция для TS/Python.** `parser/core.rs` теперь обогащает `signature` не только Rust-`attribute_item`, но и `decorator`-нодами (Python `decorated_definition` siblings, TS class-level декораторы). Покрыто тестами (`python_decorator_in_signature`, `ts_class_decorator_in_signature`).

### Оставшийся честный пробел

- **Go не верифицирован тестами** — грамматика не ставится в этом окружении (network timeout на lang-hub). Go-пресеты и union node-kinds написаны по грамматике tree-sitter-go, но `⚠️ unverified`. Как только грамматика установится — compile-тест автоматически их проверит.

## 7. Что **не** будет здесь

Сознательно за скобками — это не про код:

- Code ownership, design decisions, ADR-интеграции.
- Sentry/Bugsnag/APM-связки.
- Planning, TODO-collection, sprint-tracking.
- Issue-агрегаторы (GitHub/GitLab API).
- Code review collaboration.

Если когда-то понадобится — отдельный документ и отдельный namespace инструментов.
