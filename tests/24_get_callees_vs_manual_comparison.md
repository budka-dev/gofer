# get_callees vs manual skeleton + read

## Goal
“What does function X call?” without reading the whole file.

## Approaches

### gofer
```json
{ "symbol": "tool_search", "file": "src/daemon/handlers/search.rs" }
```
Returns structured `{name, line, resolved}` callees from outgoing edges.

### host manual
1. `rg -n 'fn tool_search'` → file  
2. read function body  
3. agent parses calls mentally  

Ops: 2–4 · tokens: full body · accuracy: depends on agent.

## Metrics (qualitative)

| Metric | gofer | host manual |
|---|---|---|
| Ops | **1** | 2–4 |
| Tokens | **low** (names only) | high (body) |
| Latency | ms | agent round-trips |
| Accuracy | high for static calls; misses macros/dyn | can see macros/strings |

## Verdict
Default to **get_callees** for navigation; host read only when edges look empty or you need macro/dyn detail.
