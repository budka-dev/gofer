# Справочник MCP-инструментов

Полный каталог инструментов, которые gofer выставляет MCP-клиенту через `tools/list` и `tools/call`. Все инструменты диспетчеризуются в `src/daemon/tools.rs::dispatch`; реализации — в `src/daemon/handlers/`.

gofer — **read-only** поисковик/навигатор. Он не изменяет файлы, не выполняет код и не поднимает внешние процессы (кроме HTTP-эмбеддера при индексации и загрузки wasm-грамматик). Мутации и рефакторинг делегируются хост-агенту.

В таблицах:

- **Обязательные параметры** выделены `**жирным**`.
- Типы — JSON Schema (`string`, `integer`, `boolean`, `array`, `object`, `number`).
- Если параметр имеет значение по умолчанию, оно указано в скобках.

## Как вызывать

Любой инструмент вызывается одинаково:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "<tool_name>",
    "arguments": { ... }
  }
}
```

Ответ возвращает MCP-content: массив `{type, text}` объектов. Большинство инструментов укладывают результат в один `text` — либо человекочитаемая строка, либо JSON-блок.

Параметр `project_path` указывать не нужно: bridge берёт его из cwd процесса `gofer mcp` (или флага `--project-dir`).

## Поиск

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `search` | **query**, `limit` (10), `path`, `glob`, `include_scores` (false), `preview_mode` (false), `min_score` (0.0), `include_context` (true) | Гибридный семантический поиск (BM25 + векторы + re-rank). `preview_mode=true` экономит ~80% токенов. |
| `search_by_purpose` | **query**, `limit` (10) | Поиск по высокоуровневому назначению (роль, ответственность). Подходит для архитектурных вопросов вроде «auth», «billing». |
| `smart_file_selection` | **query**, `limit` (5), `min_score` (0.3), `boost_recency` (0.2) | Ранжированный список файлов, релевантных задаче. Помогает выбрать, что читать дальше. |
| `search_symbols` | **query**, `kind`, `limit` (20) | Поиск символов по имени или подстроке. |
| `grep` | **pattern**, `path`, `glob` (только `*.<ext>`), `case_insensitive` (false), `context_lines` (0), `max_results` (100) | Regex по содержимому файлов. Возвращает карту совпадений по файлам. С `context_lines > 0` отдаёт строки до/после совпадения. |
| `find_unreachable` | `file` ИЛИ `path`, `limit` (200, max 2000) | Статически недостижимый код: операторы после безусловного терминатора (`return`/`break`/`continue`/`throw`/`raise`/`panic!`/`unreachable!`/`todo!`/`unimplemented!`) в том же блоке. Только прямые сиблинги — `return` внутри `if`-ветки **не** флагает код после `if`, поэтому false positives около нуля. Возвращает `unreachable[]` (с `file`/`line`/`kind`/`code`/`reason`). Вне скоупа: недостижимые match-arms после catch-all, always-false условия. |
| `complexity` | `file` ИЛИ `path`, `min_complexity` (1), `sort` (`complexity`/`lines`/`nesting`/`name`), `limit` (100, max 1000) | Цикломатическая сложность (McCabe: 1 + decision points) + size-метрики на функцию. Считает ветвления (if/elif, match/switch arms, циклы, except/catch), short-circuit операторы (`&&`/`\|\|`/`??`), тернарники. Плюс `lines`/`params`/`max_nesting`. Рейтинги: 1-5 simple, 6-10 moderate, 11-20 complex, 21+ very_complex. Вложенные замыкания идут в счёт обрамляющей функции; вложенные именованные функции — отдельные записи. Возвращает `functions[]`, `stats` (avg/max), `total_functions`. |
| `structural_search` | `preset` ∈ (см. ниже), `query` + **language**, `path`, `max_results` (200, max 2000), `include_text` (true) | Поиск по AST-форме, не regex. Без аргументов отдаёт каталог пресетов. Пресеты: **rust** — `rust_unwrap`, `rust_expect`, `rust_panic`, `rust_todo_unimplemented`, `rust_dbg`, `rust_println`, `rust_clone`; **typescript** — `ts_any`, `ts_console_log`, `ts_ts_ignore`, `ts_debugger`, `ts_non_null`; **python** — `py_print`, `py_bare_except`, `py_breakpoint`; **go** — `go_panic`, `go_fmt_print`. Custom S-expression — через `query` + `language`. Возвращает `hits[]` с `file`/`line_start`/`line_end`/`col_start`/`col_end`/`text`. Игнорирует комменты и строки на уровне синтаксиса, поэтому в разы точнее `grep`. |

## Чтение файлов

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `read_file` | **file**, `start_line` (1), `end_line` | Чтение файла с нумерацией строк, опционально с диапазоном. |
| `read_function_context` | **file**, **function**, `include_types` (true), `include_imports` (true), `include_callees` (false) | Извлекает одну функцию вместе с её зависимостями (типы, импорты, вызываемые функции). Экономит 90–95% токенов vs `read_file`. |
| `read_types_only` | **file**, `kind`, `include_docs` (true) | Только определения типов (struct, enum, interface, type alias, trait, class). |
| `skeleton` | **file**, `include_private` (false), `include_tests` (false) | Файл в режиме «скелет»: импорты, типы, сигнатуры, doc-комментарии — без тел функций. |
| `context_bundle` | **file**, `depth` (2), `skeleton` (false), `skeleton_deps_only` (false) | Рекурсивно собирает файл и его import-зависимости в один контекст. `skeleton_deps_only=true` оставляет основной файл полным, а зависимости — скелетами. |
| `get_file_metadata` | **path** | Размер, mtime, line count, бинарный/текстовый. Удобно перед чтением больших файлов. |

## Навигация

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `project_tree` | `path` (""), `depth` (3), `pattern` | Дерево каталогов с учётом `.gitignore`. |
| `list_directory` | `path` ("."), `recursive` (false), `exclude_patterns` | Список содержимого с фильтрами. По умолчанию исключает `node_modules`, `target`, `.git`, `dist`, `build`. |
| `find_files` | **pattern**, `path`, `limit` (100), `offset` (0) | Поиск файлов по glob. Возвращает поля `total`/`count`/`truncated`/`limit`/`offset`. |

## Символы и ссылки

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `get_symbols` | `file`, `kind`, `offset` (0), `limit` (200, max 500) | Список символов в файле или проекте, сгруппированный по файлам. |
| `get_references` | **symbol** | Все места, где используется символ. |
| `get_callers` | **symbol** | Входящие вызовы — кто зовёт этот символ. |
| `get_callees` | **symbol**, `file` | Исходящие вызовы — кого зовёт этот символ. |
| `symbol_exists` | **symbol**, `file` | Дешёвый булев чек по индексу. С `file` ищется внутри файла, без — глобально (`search_symbols(limit=1)`). |
| `is_exported` | **symbol**, `file` | Эвристика: смотрит сигнатуру (`pub`/`export`/имя без подчёркивания и kind ≠ LocalVar). Возвращает `is_exported` + `locations`. |
| `find_unused_symbols` | `kind`, `file`, `public_only` (true), `exclude_tests` (true), `exclude_entry_points` (true), `exclude_exports` (true), `limit` (100, max 500) | Символы без incoming `call`/`usage`/`inherit`/`type_usage` ссылок. Walks `symbol_references` (по `target_symbol_id` и `target_name`). Applied heuristics: тесты — по пути И по атрибутам в signature (`#[test]`, `#[tokio::test]`, `#[bench]`, `#[cfg(test)]`, `@pytest.fixture`) И по имени (`test_*`/`*_test`); entry points — `main`/`__main__`/`lambda_handler`/`handler`; экспорты — `#[no_mangle]`/`extern "C"`/`#[wasm_bindgen]`/`#[pyfunction]`/`#[napi]`/`@customElement`/`@Component`. **Caveats:** граф язык-агностичен; attribute/decorator-фильтрация работает для Rust (`#[test]`), Python (`@pytest.fixture`) и TS (`@Component`) — декораторы попадают в signature. Go — без attribute-маркеров (path+naming). Внешние API-потребители невидимы (`public_only=false`); trait/dyn dispatch не отслеживается; attribute-фильтрация на свежем индексе — старый требует `force_reindex`. |
| `find_by_type_signature` | `returns`, `param_type`, `signature_contains` (хотя бы один), `kind`, `file`, `limit` (100, max 500) | Поиск функций/методов по сигнатуре — region-aware. `returns` матчит **только return-зону** (не сматчит параметр того же типа), `param_type` — **только зону параметров**, `signature_contains` — raw substring по всей сигнатуре. Case-sensitive подстроки: `Result<MyType` ловит `Result<MyType, Error>`. Coarse SQL LIKE + refine через `split_signature` (первая top-level `(...)` = params, после неё `->`/`:`/bare = return). **Caveats:** Go receiver попадает в params-зону; Rust where-clause — в return-зону; нужен свежий индекс. |
| `call_path` | **from**, **to**, `direction` (`calls`/`called_by`, default `calls`), `file_from`, `file_to`, `max_depth` (8, max 30), `max_paths` (5, max 50) | BFS поверх `symbol_references` между двумя символами. `calls` — `from` транзитивно вызывает `to` через outgoing edges; `called_by` — `from` достижим из `to` через incoming. Возвращает `paths[]` (каждый — список `{symbol, kind, file, line, incoming_edge}`), `shortest_length`. Unresolved refs резолвятся через `get_symbol_by_name` (cap 16 кандидатов на ребро). **Caveats:** один shortest path на цель, alternatives не перечисляются; dyn/trait dispatch не моделируется; на общих именах вроде `new` могут быть ложные ветки. |
| `dependency_subgraph` | **symbol**, `file`, `direction` (`out`/`in`/`both`, default `out`), `max_depth` (3, max 20), `max_nodes` (50, max 500) | BFS-окрестность символа по `symbol_references`. `out` — зависимости, `in` — зависимые, `both` — объединение. Возвращает `nodes[]` + `edges[]` в пределах глубины. В отличие от `call_path` (путь к цели) — весь достижимый подграф, для impact-анализа и «blast radius». Edges с обрезанными при truncation концами отбрасываются. |
| `find_implementations` | **name**, `limit` (100, max 500) | Реализации trait/interface/base по имени, кросс-язычно. Rust `impl <Name> for <Type>` (только trait-позиция — тип не сматчится), TS `implements`/`extends`, Python базовые классы. Whole-token матчинг (`Foo` ≠ `FooBar`). Возвращает `implementations[]` (`type`/`kind`/`relation`/`file`/`line`). **Go не поддержан** — satisfaction интерфейсов структурный, не декларируется. |
| `find_unused_imports` | **file**, `include_reexports` (false) | Импорты в файле, локальное имя которых нигде в этом же файле не используется. Парсит импорты через tree-sitter (Rust/TS/Python/Go), word-boundary regex по строкам **вне импорт-зоны**. Skips wildcards (`use foo::*`, `from foo import *`, Go `_`/`.` импорты). По умолчанию skip `pub use` (re-exports) — переключить через `include_reexports=true`. Возвращает `unused_imports[]` (с `name`/`source`/`line`), `total_items_checked`, `total_unused`, `skipped_wildcards`, `skipped_reexports`. **Caveats:** false positives, если идентификатор используется только через макрос-склейку (`paste!`, `concat_idents!`); false negatives, если локальное имя совпадает с не-связанным методом другого типа. |

## Зависимости проекта

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `get_dependencies` | `ecosystem` (`cargo` / `npm`) | Зависимости из `Cargo.toml`/`package.json` с версиями. |
| `dependency_impact` | **name** | Файлы, которые используют указанную зависимость. |

## Git

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `git_diff` | `file`, `staged` (false) | Diff staged или unstaged изменений (опционально по файлу). |
| `git_blame` | **file**, `line`, `start_line`, `end_line` | Blame для строки или диапазона. По одному entry на blame-hunk, с `lines_in_hunk`. |
| `git_history` | **file**, `limit` (10) | Последние коммиты, которые трогали файл. |
| `suggest_commit` | `style` (`conventional`), `include_emoji` (true), `max_subject_length` (72) | Анализирует diff и предлагает commit message. Поддерживает `conventional`/`simple`/`detailed`. |

## Метаданные проекта

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `domain_stats` | — | Сводка по доменам (`backend`/`frontend`/`ops`/…): количество файлов на каждый домен. |
| `get_vue_tree` | **file** | Иерархия Vue-компонента (template tree), сохранённая при индексации. Возвращает `tree` или `null` с сообщением, если файл не проиндексирован. |
| `get_config_keys` | — | Конфиг-ключи проекта из индекса: `key (type) src:source [required]`. |

## Индекс и обслуживание

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `get_index_status` | — | Статус индекса: completeness, число файлов/чанков, время последнего sync. |
| `validate_index` | — | Сравнивает диск с SQLite, помечает расхождения между реальными файлами и индексом. |
| `force_reindex` | `scope` (`file`/`directory`/`project`, по умолчанию `file`), `path` | Принудительная переиндексация выбранного скоупа. |
| `get_cache_stats` | — | Hit-rate и размер серверного LRU. |
| `get_query_stats` | — | Статистика поисковых запросов (latency, частоты). |
| `has_tests_for` | **file** | Проверяет на диске типовые шаблоны тестов: `*.test.{ts,js}`, `*.spec.{ts,js}`, `*_test.rs`, `test_*`, `*_test.py`, `tests/test_*`, `__tests__/*`. Возвращает `has_tests`, `test_files`, `count`. |

## Здоровье

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `health_check` | — | Проверяет sqlite, lance, эмбеддер. Возвращает подробный статус по каждому компоненту. |

## Batch

| Инструмент | Аргументы | Что делает |
|---|---|---|
| `batch_operations` | **operations** (`type` ∈ `read_file`/`get_symbols`/`search`/`skeleton`, `params`), `parallel` (true), `continue_on_error` (true), `summary_only` (false), `max_chars_per_op` | Выполняет до N read/search-операций в одном запросе. Параллелит, чтобы сократить latency в 3–5×. `summary_only=true` отдаёт только статусы; `max_chars_per_op` отрезает каждую `data` до лимита и добавляет `data_truncated`/`data_full_chars`. |

## Где смотреть свежий список

- `src/daemon/tools.rs::dispatch` — единственная точка диспетчеризации; новый инструмент должен быть упомянут здесь.
- `src/daemon/tools.rs::core_tools_list` — канонический JSON-Schema для каждого инструмента (то, что отдаётся клиенту через `tools/list`).
- Конкретные реализации — в `src/daemon/handlers/<group>.rs` (см. таблицу в [architecture.md](architecture.md#маршрутизация-mcp-инструментов)).
