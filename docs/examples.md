> **Note (2026-07):** surface trimmed to ~16 index-search tools. Examples mentioning `grep`, `read_file`, `structural_search`, `smart_file_selection`, git tools are historical — use host tools for FS/grep; gofer for search/skeleton/refs.

# Примеры использования

Этот документ — про то, **как работать с gofer'ом в реальных сценариях**. Команды и JSON-RPC вызовы взяты с рабочего инстанса, не из головы. Если хочешь понять архитектуру — иди в [architecture.md](architecture.md). Если ищешь параметры конкретного инструмента — в [tools-reference.md](tools-reference.md).

Все примеры предполагают:

- Демон поднят (`gofer up`).
- Проект зарегистрирован (`gofer init`) и проиндексирован (`gofer start`).
- HTTP-эмбеддер крутится по дефолтному адресу.

## Подключение к Claude Code

1. Установи gofer: `cargo install --path . --locked`.
2. Запусти эмбеддер (см. [embedder.md](embedder.md)).
3. Подними демон один раз: `gofer up`.
4. Зайди в каталог проекта и активируй: `cd /path/to/project && gofer init && gofer start`.
5. В Claude Code открой настройки MCP (через UI или редактируя `~/.claude.json`) и добавь:

   ```json
   {
     "mcpServers": {
       "gofer": {
         "command": "/home/<user>/.cargo/bin/gofer",
         "args": ["mcp"]
       }
     }
   }
   ```

6. Перезапусти Claude Code. Инструменты gofer'а появятся под префиксом `mcp__gofer__*`.

`gofer mcp` сам разберётся, в каком каталоге запущен (cwd клиента) и подключится к этому проекту. Если запускаешь Claude Code из другого каталога — передай `--project-dir`:

```json
"args": ["mcp", "--project-dir", "/path/to/project"]
```

## Подключение к Qoder и другим MCP-клиентам

Любой клиент, понимающий MCP stdio, конфигурируется одинаково — это `command` + `args`. Структура `.mcp.json` или эквивалент в Qoder/Cursor/Continue одна и та же. Замени `/home/<user>/.cargo/bin/gofer` на абсолютный путь к бинарю.

## Сценарий 1. Найти и понять функцию

**Задача:** ассистенту нужно объяснить, как работает функция `dispatch` в `daemon/tools.rs`.

Наивный путь — `read_file` для всего файла (1444 строки → ~20 000 токенов).

**Эффективный путь через gofer:**

```
1. search "tool dispatch routing"
   → находит daemon/tools.rs:dispatch, src/main.rs handle_search

2. read_function_context file="src/daemon/tools.rs" function="dispatch"
   → ~200 строк: сама функция + типы (Value, ToolContext) + используемые импорты
   → токенов: ~1500 вместо 20000
```

Если нужны ещё и реализации зависимых функций:

```
read_function_context file="src/daemon/tools.rs" function="dispatch" include_callees=true
```

Это вытащит на 1 уровень глубже все вызываемые функции (search::tool_search, files::tool_read_file, …).

## Сценарий 2. Понять файл целиком, не читая его

**Задача:** ассистент видит, что задействован `src/indexer/pipeline.rs` (1100 строк), но детали ему не нужны — нужна структура.

```
1. skeleton file="src/indexer/pipeline.rs"
   → только сигнатуры, типы, doc-комментарии
   → ~150 строк вместо 1100
```

Если файл нужен с зависимостями (например, чтобы понять контекст):

```
2. context_bundle file="src/indexer/pipeline.rs" depth=2 skeleton_deps_only=true
   → сам pipeline.rs полностью, его import-зависимости в виде скелетов
```

## Сценарий 3. Найти, кто вызывает функцию

**Задача:** хочешь зарефакторить `EmbedderPool::embed`, но боишься сломать вызывающих.

```
1. get_callers symbol="embed"
   → список всех файлов и строк с вызовами

2. (для точного blast-radius) dependency_subgraph symbol="embed" direction="in"
   → BFS-граф входящих зависимостей с глубиной до N уровней
```

`get_callers` работает из индекса (быстро, токено-экономно). `dependency_subgraph` строит более полную картину для сложных случаев.

## Сценарий 4. Сделать N запросов за один round-trip

**Задача:** ассистент хочет получить структуру файла, прочитать конкретный фрагмент и поискать упоминания одной строки. Это 3 отдельных вызова MCP.

```
batch_operations operations=[
  {"type": "get_symbols", "params": {"file": "src/daemon/state.rs"}},
  {"type": "read_file",   "params": {"file": "src/daemon/state.rs", "start_line": 22, "end_line": 80}},
  {"type": "search",      "params": {"query": "embedding_circuit", "limit": 5}}
] parallel=true
```

Все три уходят в работу параллельно, ответ приходит одним сообщением. На больших latency-чувствительных сценариях экономит 50–80% времени.

Опции:

- `continue_on_error=true` — не валиться целиком, если один шаг не сработал (по умолчанию).
- `summary_only=true` — отдать только статусы (полезно для health-чека).
- `max_chars_per_op=8000` — резать каждый ответ, чтобы не разорвать контекст.

## Сценарий 5. Smart-коммит

**Задача:** ассистент готов закоммитить N изменённых файлов и хочет нормальное сообщение.

```
1. git_diff           (увидеть, что меняется)
2. suggest_commit style=conventional include_emoji=false
   → "feat(indexer): add embedding timeout to 30s"
3. (создать коммит уже через стандартный git/bash)
```

`suggest_commit` смотрит diff staged + unstaged, анализирует, что добавлено/удалено/изменено, и предлагает заголовок + тело в Conventional Commits.

## Сценарий 6. Только типы из файла

**Задача:** нужно понять модель данных модуля, не залезая в логику.

```
read_types_only file="src/daemon/state.rs"
   → DaemonState, DaemonMetrics, SyncProgress, ProjectState, SyncProgressSnapshot
   → только структуры/енумы, без impl-блоков методов
```

Сильно дешевле, чем `read_file`, и точнее, чем `skeleton` (в skeleton сигнатуры функций тоже остаются).

## Сценарий 7. «Какие файлы важны для задачи X»

**Задача:** ассистент не знает, с чего начать. Хочет ранжированный список релевантных файлов.

```
smart_file_selection
  query="как работает graceful shutdown демона"
  limit=5
  min_score=0.3
```

Возвращает 5 файлов с оценкой релевантности. Дальше — `skeleton` по топ-3, и только потом `read_file` по тому, что нужно.

Параметр `boost_recency` (0..1) добавляет вес недавним правкам — полезно, если задача касается активной разработки.

## Сценарий 8. Полная диагностика проекта

Когда что-то «не так» и непонятно где:

```
1. get_index_status           # completeness, число файлов/чанков
2. validate_index             # расхождения диск ↔ SQLite
3. health_check               # sqlite/lance/embedder
4. get_cache_stats            # hit-rate серверного LRU
5. get_query_stats            # latency поиска
```

Если `validate_index` показывает несоответствия — `force_reindex scope=project` восстанавливает консистентность. Снос диска (`rm -rf .gofer/data/`) — последняя инстанция.

## Сценарий 9. Структурный поиск по AST-паттернам

**Задача:** найти все вызовы `.unwrap()` в Rust-коде без ложных срабатываний из комментариев и строк.

```
1. structural_search preset="rust_unwrap" path="src/"
   → только реальные .unwrap() вызовы, без false positives из комментариев

2. structural_search preset="rust_todo_unimplemented"
   → todo!() и unimplemented!() по всему проекту

3. structural_search query="(call_expression
     function: (field_expression field: (field_identifier) @name
       (#eq? @name \"unwrap\")))" language="rust"
   → custom S-expression, то же, но полностью под твой контроль
```

`structural_search` в разы точнее `grep` — работает на уровне AST, а не текста.

## Сценарий 10. Поиск реализаций trait/interface

**Задача:** найти все типы, реализующие `EmbedderTrait`.

```
1. find_implementations name="EmbedderTrait"
   → list всех Rust impl, TS implements, Python базовых классов

2. (для более широкого контекста) search "impl EmbedderTrait"
   → семантический поиск в окрестности
```

## Анти-паттерны

Чего стоит избегать:

- **`read_file` для целого файла >300 строк, если не нужны все детали.** Используй `skeleton` / `read_function_context` / `read_types_only`.
- **`search` без `min_score`.** Низкокачественные совпадения с score 0.1 будут мусорить контекст.
- **Сериализация инструментов, когда можно параллелить.** `batch_operations` экономит latency.
- **Игнорирование `get_index_status` перед поиском после крупного git pull.** Индекс может быть устаревшим, watcher отрабатывает не мгновенно.

## Дальше

- [tools-reference.md](tools-reference.md) — полный каталог инструментов.
- [architecture.md](architecture.md) — как это всё работает внутри.
- [troubleshooting.md](troubleshooting.md) — что делать, когда не работает.
