> **Note (2026-07):** gofer is a **16-tool index-search MCP only** (hybrid search, symbols/refs/callers, skeleton / function_context / types / bundle, batch, index ops). It is **not** a host FS/grep/git/edit/execute layer.
>
> Scenarios or recipes that mention `read_file`, `grep`, `structural_search`, `smart_file_selection`, `suggest_commit`, `git_*`, `dependency_subgraph`, `health_check`, `get_cache_stats`, etc. are **HISTORICAL** — use host-agent tools for those jobs; use gofer for search / compact read / graph / index. Canonical list: [tools-reference.md](tools-reference.md).

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

Наивный путь — полный `read_file` хоста для всего файла (тысячи строк → десятки тысяч токенов).

**Эффективный путь через gofer:**

```
1. search "tool dispatch routing"
   → находит daemon/tools.rs:dispatch

2. read_function_context file="src/daemon/tools.rs" function="dispatch"
   → сама функция + типы (Value, ToolContext) + используемые импорты
   → токенов: порядка 10× меньше полного файла
```

Если нужны ещё и реализации зависимых функций:

```
read_function_context file="src/daemon/tools.rs" function="dispatch" include_callees=true
```

Это вытащит на 1 уровень глубже вызываемые handler'ы (`search::tool_search`, `symbols::…`, …).

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

2. get_references symbol="embed"
   → все использования имени (шире, чем только call)

3. get_callees symbol="embed" file="src/indexer/embedder.rs"
   → кого вызывает сама функция (blast radius наружу)
```

`get_callers` / `get_references` / `get_callees` работают из индекса (быстро, токено-экономно).  
~~`dependency_subgraph`~~ — **HISTORICAL** (снят с surface); для глубокого BFS используй host + несколько graph tools.

## Сценарий 4. Сделать N запросов за один round-trip

**Задача:** ассистент хочет получить структуру файла, скелет и поискать упоминания одной строки. Это 3 отдельных вызова MCP.

```
batch_operations operations=[
  {"type": "get_symbols", "params": {"file": "src/daemon/state.rs"}},
  {"type": "skeleton",    "params": {"file": "src/daemon/state.rs"}},
  {"type": "search",      "params": {"query": "embedding_circuit", "limit": 5}}
] parallel=true
```

Все три уходят в работу параллельно, ответ приходит одним сообщением. На latency-чувствительных сценариях экономит round-trips.

Допустимые `type` в batch: `search` | `get_symbols` | `skeleton` | `get_references` | `read_function_context` | `read_types_only` (см. tools-reference).

## ~~Сценарий 5. Smart-коммит~~ — HISTORICAL

> **Removed from MCP surface.** `git_diff` / `suggest_commit` больше не в 16-tool index-search set. Diff и commit message — через host `git` / agent tools.

```
# was:
# 1. git_diff
# 2. suggest_commit style=conventional
# 3. git commit …
```

## Сценарий 6. Только типы из файла

**Задача:** нужно понять модель данных модуля, не залезая в логику.

```
read_types_only file="src/daemon/state.rs"
   → DaemonState, DaemonMetrics, SyncProgress, ProjectState, SyncProgressSnapshot
   → только структуры/енумы, без impl-блоков методов
```

Сильно дешевле полного чтения файла хостом, и точнее, чем `skeleton` (в skeleton сигнатуры функций тоже остаются).

## ~~Сценарий 7. «Какие файлы важны для задачи X»~~ — HISTORICAL

> **`smart_file_selection` removed.** Use hybrid `search` (+ optional `min_score` / `include_scores`), then `skeleton` on top hits.

```
search query="как работает graceful shutdown демона" limit=10 include_scores=true
→ skeleton file=<top hit>
```

## Сценарий 8. Диагностика индекса

Когда поиск «пустой» или индекс выглядит устаревшим:

```
1. get_index_status           # completeness, sync age, embedder probe
2. validate_index             # integrity issues + recommendations
3. reindex force=true         # full_sync + resolve_references (or path= for one file)
```

~~`health_check` / `get_cache_stats` / `get_query_stats`~~ — **HISTORICAL** (не в core list). CLI: `gofer health` / `gofer status`. Снос диска (`rm -rf .gofer/data/`) — последняя инстанция.

## ~~Сценарий 9. Структурный поиск по AST-паттернам~~ — HISTORICAL

> **`structural_search` removed** from MCP surface. AST-precise unwrap/todo hunts: host `rg` / tree-sitter tooling. Semantic / symbol search remains via `search` / `search_symbols`.

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

- **Полный host `read_file` для файла >300 строк, если не нужны все детали.** Используй gofer `skeleton` / `read_function_context` / `read_types_only`.
- **`search` без фильтра качества на шумных запросах.** При необходимости `min_score` / `include_scores` + меньший `limit`.
- **Сериализация инструментов, когда можно параллелить.** `batch_operations` экономит latency.
- **Игнорирование `get_index_status` перед поиском после крупного git pull.** Индекс может быть устаревшим, watcher отрабатывает не мгновенно.
- **Ожидание FS/grep/git/edit tools от gofer.** Их нет — 16 index-search tools only.

## Дальше

- [tools-reference.md](tools-reference.md) — полный каталог инструментов.
- [architecture.md](architecture.md) — как это всё работает внутри.
- [troubleshooting.md](troubleshooting.md) — что делать, когда не работает.
