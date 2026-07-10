# gofer product plan — index search MCP only

**Status:** 16-tool surface shipped; ref precision + embedder notes in progress.

**Principle:** index + search + compact read. Host agent owns FS, grep, git, edits, answers.  
**Anti-goal:** replace host-agent tools.

## Target MCP surface (~14 tools)

| Tool | Why keep |
|---|---|
| `search` | Hybrid semantic — unique |
| `search_symbols` | Name/substring symbol search |
| `get_symbols` | File/project symbol listing |
| `get_references` | Index graph |
| `get_callers` / `get_callees` | Index graph |
| `find_implementations` | Index |
| `find_by_type_signature` | Index |
| `skeleton` | Compact AST read |
| `read_function_context` | Compact AST read |
| `read_types_only` | Compact AST read |
| `context_bundle` | Compact multi-file |
| `batch_operations` | Latency (search/symbols/skeleton only) |
| `get_index_status` | Index ops |
| `validate_index` | Index ops |
| `reindex` | Index ops (new thin MCP) |

## Drop (host-duplicate / micro / answer)

- FS: `read_file`, `grep`, `find_files`, `list_directory`, `project_tree`, `get_file_metadata`, `file_exists`
- Micro: `symbol_exists`
- Already gone: analysis, composite, git, etc.
- MCP prompts: remove (answer assembly)
- Resources: keep only if trivial or remove tree; stats via tools if needed

## Hygiene

1. Migration drop dead tables (deps, rules, vue, fingerprints, …)
2. Docs sync to 14 tools; archive superpowers specs as wontfix
3. validate_index points to `reindex` tool / CLI
4. Stop writing domain-only noise if unused (optional keep domain column write)

## Later

- [x] Reference resolution precision (`prefer_resolved` + optional `file` on get_references/get_callers)
- [ ] Unified response envelope (still mixed flat strings vs JSON objects)
- [x] Embedder DX / offline notes in tools-reference
- [ ] Comparison tests refresh (`tests/*_comparison.md` still mention removed tools)

## Execution order

1. Drop tools + handlers usage + batch + resources/prompts  
2. Add `reindex`  
3. DB migration  
4. Docs  
5. check/test/commit  
