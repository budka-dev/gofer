# Сравнительные отчёты (index-search surface)

Не unit-тесты, а замеры **token/latency** gofer tools vs host-native alternatives.  
Unit-тесты: `#[cfg(test)]` в `src/` — см. [docs/development.md](../docs/development.md).

## Позиционирование

gofer = **index search + compact read** (16 MCP tools).  
Хост-агент = FS, grep/rg, git, edits, shell.

Сравнения для **снятых** tools (read_file, grep tool, write, sandbox, …) лежат в:

- `tests/archive/host-duplicate-removed/`
- `tests/archive/mutation-sandbox-removed/`

## Методология

[00_methodology.md](00_methodology.md) — метрики Accuracy / tokens / latency.

## Актуальный каталог

| Файл | Сравнение | Зачем gofer |
|---|---|---|
| [02_search_vs_grep_comparison.md](02_search_vs_grep_comparison.md) | semantic `search` vs `rg` | смысл, не только regex |
| [03_skeleton_vs_read_comparison.md](03_skeleton_vs_read_comparison.md) | `skeleton` vs full file | 3–5× tokens |
| [04_get_symbols_vs_grep_comparison.md](04_get_symbols_vs_grep_comparison.md) | `get_symbols` vs grep fn/struct | AST kinds |
| [07_batch_operations_comparison.md](07_batch_operations_comparison.md) | batch vs N sequential | latency |
| [13_context_bundle_comparison.md](13_context_bundle_comparison.md) | bundle vs manual imports | multi-file context |
| [15_read_function_context_comparison.md](15_read_function_context_comparison.md) | function context vs full file | 90%+ tokens |
| [16_read_types_only_comparison.md](16_read_types_only_comparison.md) | types only vs full file | data-model focus |
| [18_search_symbols_comparison.md](18_search_symbols_comparison.md) | `search_symbols` vs grep/ctags | name index |
| [23_get_references_vs_grep_comparison.md](23_get_references_vs_grep_comparison.md) | refs/callers vs `rg` | resolved graph |
| [24_get_callees_vs_manual_comparison.md](24_get_callees_vs_manual_comparison.md) | callees vs read body | 1-op nav |
| [25_find_implementations_comparison.md](25_find_implementations_comparison.md) | impls vs grep | whole-token |

**Ещё нет:** `find_by_type_signature`, `reindex` / `validate_index` dedicated reports.

## MCP response envelope (2026-07)

`tools/call` text payload:

```json
{
  "ok": true,
  "tool": "search",
  "result": { /* tool-specific */ },
  "meta": { "latency_ms": 12 }
}
```

On failure: `ok: false`, `error.message`, same `meta`. Symbols/refs/search `result` items are structured objects (`file`/`line`/…), not free-form strings.

## Как использовать

Новый tool или оптимизация — добавь `tests/NN_<tool>_comparison.md` по методологии. Не сравнивай с host-tools, которые gofer **намеренно** не дублирует.

## Live self-bench (golden queries)

Quick smoke against a healthy daemon on **this** repo (index should be warm):

```bash
./scripts/self_bench.sh
# single-query micro-bench:
./scripts/bench_live.sh dispatch
```

`self_bench.sh` runs ≥5 golden searches (`dispatch`, `tool_search`, `resolve_references`, `skeleton`, `full_sync`), prints `wall_ms` per query, hard-fails on search crash, soft-warns if the first hit path misses an expected substring.
