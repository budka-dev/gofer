# Справочник MCP-инструментов

Каталог tools, которые gofer отдаёт через `tools/list` / `tools/call`.  
Канон: `src/daemon/tools.rs` (`dispatch` + `core_tools_list`).

## Позиционирование

gofer — **read-only index-search MCP**, не замена native tools агента.

| gofer | Хост-агент (Claude Code / Cursor / …) |
|---|---|
| Семантический поиск, символы, refs/callers | `grep`, `find`, FS tree |
| skeleton / function context / types / bundle | полный `read_file` |
| reindex / validate index | git, edits, tests, shell |

Мутации, grep, list dir, git, lint, execute — **не входят** в gofer.

Сейчас **16 tools**.

## Response envelope

Каждый `tools/call` возвращает MCP `content[].text` — JSON:

```json
{
  "ok": true,
  "tool": "search",
  "result": { },
  "meta": { "latency_ms": 12 }
}
```

Ошибка:

```json
{
  "ok": false,
  "tool": "reindex",
  "error": { "message": "..." },
  "meta": { "latency_ms": 3 }
}
```

Внутри `result` — данные инструмента. Списки символов/ссылок/search hits — **объекты** (`file`, `line`, …), не склеенные строки.

## Поиск

| Tool | Args | Что делает |
|---|---|---|
| `search` | **query**, `limit`, `path`, `glob`, `include_scores`, `preview_mode`, `min_score`, `include_context`, `max_per_file` (3) | Hybrid vector+FTS. Exact symbol tokens boosted; diversify by file; content capped ~600 chars. |
| `search_symbols` | **query**, `kind`, `limit` | Символы по имени/подстроке. |

## Символы и граф

| Tool | Args | Что делает |
|---|---|---|
| `get_symbols` | `file`, `kind`, `offset`, `limit` | Список символов файла/проекта. |
| `get_references` | **symbol** | Все использования имени. |
| `get_callers` | **symbol** | Кто вызывает. |
| `get_callees` | **symbol**, `file` | Кого вызывает. |
| `find_implementations` | **name**, `limit` | Impl trait/interface/base. |
| `find_by_type_signature` | `returns` / `param_type` / `signature_contains`, … | Поиск по сигнатуре (region-aware). |

## Компактное чтение

| Tool | Args | Что делает |
|---|---|---|
| `skeleton` | **file**, `include_private`, `include_tests` | Сигнатуры без тел (3–5× меньше). |
| `read_function_context` | **file**, **function**, `include_types`, `include_imports`, `include_callees` | Одна функция + deps. |
| `read_types_only` | **file**, `kind`, `include_docs` | Только типы. |
| `context_bundle` | **file**, `depth`, `skeleton`, `skeleton_deps_only` | Файл + import-deps. |

## Batch и индекс

| Tool | Args | Что делает |
|---|---|---|
| `batch_operations` | **operations** (`search` \| `get_symbols` \| `skeleton` \| `get_references` \| `read_function_context` \| `read_types_only`), `parallel`, … | Несколько index-ops за один RPC. |
| `get_index_status` | — | Completeness, embedder health probe, ref resolve %, sync age. |
| `validate_index` | — | Integrity issues + recommendations. |
| `reindex` | `force`, `path` | `path` — один файл + resolve; `force=true` — clear + **full_sync** + resolve (один call). |

## Resources

| URI | Содержание |
|---|---|
| `project://stats` | `files` count + symbols by kind |

MCP **prompts** сняты — агент сам собирает ответ из tools.

## См. также

- [PRODUCT_PLAN.md](PRODUCT_PLAN.md) — план продукта  
- [examples.md](examples.md) — сценарии (частично устаревают — правь под 16 tools)  

## Embedder / offline behaviour

gofer does **not** embed locally. It POSTs batches to an HTTP embed endpoint (default `http://127.0.0.1:8080/embed/`).

| Mode | Behaviour |
|---|---|
| Embedder up | Full hybrid `search` (vector + FTS) |
| Embedder down | `search` degrades to keyword/FTS where possible and may set `degraded`/warnings; symbol tools and compact read keep working |
| No index / empty project | MCP `reindex force=true` (full SQLite+Lance rebuild) or `gofer start` |

## Force reindex

`reindex force=true` clears **SQLite** symbol tables **and** the **Lance** `code_chunks` table, then runs full_sync + resolve_references. Orphan embeddings cannot survive.

After updating tree-sitter query packs under `~/.gofer/langs/*/queries/`, run force reindex so new `@inherit` / `@type_usage` edges are captured.

## Live micro-bench

```bash
./scripts/bench_live.sh dispatch          # single query
./scripts/self_bench.sh                   # golden queries (dispatch, tool_search, …)
```

Config: project `.gofer` / embed URL (see [config-reference.md](config-reference.md) if present).
