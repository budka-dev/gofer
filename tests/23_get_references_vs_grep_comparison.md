# get_references / get_callers vs host grep

## Goal
Compare **index graph** tools with host `rg` for “where is this used?”.

## Product stance
- gofer: resolved/name edges from the AST index (`prefer_resolved` + optional `file`).
- host: text search — fast, but no symbol binding.

## Scenario A — unique function name

**Symbol:** `tool_search` (expect few call sites in gofer itself)

| Approach | Ops | Typical tokens | Notes |
|---|---|---|---|
| `get_references {symbol:"tool_search"}` | 1 | low (structured list) | prefers `target_symbol_id` |
| `get_callers {symbol:"tool_search"}` | 1 | low | call/usage only |
| `rg -n 'tool_search'` | 1 | medium | includes defs, strings, comments |

**Accuracy (qualitative):** gofer ≥ host when index is fresh — fewer string/comment hits.  
**When host wins:** symbol not indexed yet; need raw text.

## Scenario B — common name (`new`, `parse`, `handle`)

| Approach | Risk |
|---|---|
| `get_references {symbol:"new"}` without `file` | many defs; even resolved edges fan out |
| `get_references {symbol:"new", file:"src/foo.rs"}` | scoped to that definition’s id |
| `rg '\bnew\b'` | huge noise |

**Accuracy:** gofer + `file` clearly better. Without `file`, both noisy; gofer still better if resolve_references ran.

## Scenario C — after large rename / no reindex

| Approach | Behaviour |
|---|---|
| gofer refs | stale until `reindex path=` / watcher |
| `rg` | current disk truth |

**Ops:** gofer may need `reindex` + refs (2 ops) vs 1 `rg`.

## Token / latency (order of magnitude)

| Tool | Latency (warm daemon) | Response shape |
|---|---|---|
| get_references | ~1–10 ms | envelope + `{file,line,kind}[]` |
| get_callers | ~1–10 ms | same filter call/usage |
| rg | ~5–50 ms | line text, no kinds |

## Verdict

| Task | Prefer |
|---|---|
| “Who calls this definition?” | **gofer** `get_callers` (+ `file` if ambiguous) |
| “Any text mention?” | **host** `rg` |
| “Callees of this fn” | **gofer** `get_callees` (outgoing index edges) |

## Caveats
- dyn/trait dispatch incomplete  
- unresolved edges fall back to name match  
- needs successful index + resolve_references pass  
