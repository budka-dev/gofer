# Справочник MCP-инструментов

Полный каталог инструментов, которые gofer выставляет MCP-клиенту через `tools/list` и `tools/call`. Все инструменты диспетчеризуются в `src/daemon/tools.rs::dispatch`; реализации — в `src/daemon/handlers/`.

gofer — **read-only** поисковик/навигатор. Он не изменяет файлы, не выполняет код и не поднимает внешние процессы (кроме HTTP-эмбеддера при индексации и загрузки wasm-грамматик). Мутации и рефакторинг делегируются хост-агенту.

Поверхность инструментов сужена до **индекса + поиска + компактного чтения**. Analysis/lint/impact/graph-BFS и answer-helpers сняты: сборку ответа делает хост-агент.


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
| `search_symbols` | **query**, `kind`, `limit` (20) | Поиск символов по имени или подстроке. |
| `grep` | **pattern**, `path`, `glob` (только `*.<ext>`), `case_insensitive` (false), `context_lines` (0), `max_results` (100) | Regex по содержимому файлов. Возвращает карту совпадений по файлам. С `context_lines > 0` отдаёт строки до/после совпадения. |

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
| `find_by_type_signature` | `returns`, `param_type`, `signature_contains` (хотя бы один), `kind`, `file`, `limit` (100, max 500) | Поиск функций/методов по сигнатуре — region-aware. `returns` матчит **только return-зону** (не сматчит параметр того же типа), `param_type` — **только зону параметров**, `signature_contains` — raw substring по всей сигнатуре. Case-sensitive подстроки: `Result<MyType` ловит `Result<MyType, Error>`. Coarse SQL LIKE + refine через `split_signature` (первая top-level `(...)` = params, после неё `->`/`:`/bare = return). **Caveats:** Go receiver попадает в params-зону; Rust where-clause — в return-зону; нужен свежий индекс. |
| `find_implementations` | **name**, `limit` (100, max 500) | Реализации trait/interface/base по имени, кросс-язычно. Rust `impl <Name> for <Type>` (только trait-позиция — тип не сматчится), TS `implements`/`extends`, Python базовые классы. Whole-token матчинг (`Foo` ≠ `FooBar`). Возвращает `implementations[]` (`type`/`kind`/`relation`/`file`/`line`). **Go не поддержан** — satisfaction интерфейсов структурный, не декларируется. |

## Зависимости проекта

| Инструмент | Аргументы | Что делает |
|---|---|---|

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
