//! Core MCP tool implementations — extracted from mcp.rs for shared use by daemon.

use anyhow::Result;
use serde_json::{json, Value};

use super::handlers::*;
use crate::error::GoferError; // Import all handlers modules

// Re-export ToolContext so it's available as crate::daemon::tools::ToolContext
pub use super::handlers::common::ToolContext;

/// Dispatch a tool call by name. Returns structured JSON.
pub async fn dispatch(name: &str, args: Value, ctx: &ToolContext) -> Result<Value> {
    match name {
        "search" => search::tool_search(args, ctx).await,
        "get_symbols" => symbols::tool_get_symbols(args, ctx).await,
        "get_references" => symbols::tool_get_references(args, ctx).await,
        "get_dependencies" => project::tool_get_dependencies(args, ctx).await,
        "dependency_impact" => project::tool_dependency_impact(args, ctx).await,
        "get_config_keys" => diagnostics::tool_get_config_keys(ctx).await,
        "get_vue_tree" => project::tool_get_vue_tree(args, ctx).await,
        "git_blame" => git::tool_git_blame(args, ctx).await,
        "git_history" => git::tool_git_history(args, ctx).await,
        "context_bundle" => files::tool_context_bundle(args, ctx).await,

        "domain_stats" => project::tool_domain_stats(ctx).await,

        "search_by_purpose" => search::tool_search_by_purpose(args, ctx).await,
        "structural_search" => structural::tool_structural_search(args, ctx).await,
        "complexity" => complexity::tool_complexity(args, ctx).await,
        "find_unreachable" => unreachable::tool_find_unreachable(args, ctx).await,
        "skeleton" => files::tool_skeleton(args, ctx).await,
        "read_file" => files::tool_read_file(args, ctx).await,
        "project_tree" => project::tool_project_tree(args, ctx).await,
        "search_symbols" => symbols::tool_search_symbols(args, ctx).await,
        "grep" => files::tool_grep(args, ctx).await,
        "find_files" => files::tool_find_files(args, ctx).await,
        "git_diff" => git::tool_git_diff(args, ctx).await,
        "get_callers" => symbols::tool_get_callers(args, ctx).await,
        "get_callees" => symbols::tool_get_callees(args, ctx).await,
        "health_check" => diagnostics::tool_health_check(ctx).await,
        // Phase 0: Index Quality & Token Efficiency
        "get_index_status" => index::tool_get_index_status(ctx).await,
        "validate_index" => index::tool_validate_index(ctx).await,
        "force_reindex" => index::tool_force_reindex(args, ctx).await,
        "file_exists" => files::tool_file_exists(args, ctx).await,
        "symbol_exists" => symbols::tool_symbol_exists(args, ctx).await,
        "has_tests_for" => diagnostics::tool_has_tests_for(args, ctx).await,
        "is_exported" => symbols::tool_is_exported(args, ctx).await,
        "find_unused_symbols" => symbols::tool_find_unused_symbols(args, ctx).await,
        "find_unused_imports" => files::tool_find_unused_imports(args, ctx).await,
        "find_by_type_signature" => symbols::tool_find_by_type_signature(args, ctx).await,
        "find_implementations" => symbols::tool_find_implementations(args, ctx).await,
        "call_path" => symbols::tool_call_path(args, ctx).await,
        "dependency_subgraph" => symbols::tool_dependency_subgraph(args, ctx).await,
        "suggest_commit" => git::tool_suggest_commit(args, ctx).await,
        "get_cache_stats" => index::tool_get_cache_stats(ctx).await,
        "get_query_stats" => index::tool_get_query_stats(ctx).await,
        "read_function_context" => files::tool_read_function_context(args, ctx).await,
        "read_types_only" => files::tool_read_types_only(args, ctx).await,
        "smart_file_selection" => search::tool_smart_file_selection(args, ctx).await,
        "batch_operations" => batch::tool_batch_operations(args, ctx).await,
        // Phase 1: File Operations
        "list_directory" => file_ops::tool_list_directory(args, ctx).await,
        "get_file_metadata" => file_ops::tool_get_file_metadata(args, ctx).await,
        _ => Err(GoferError::MethodNotFound(name.to_string()).into()),
    }
}

/// Return the static list of core tools (no language-service tools).
pub fn core_tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "search",
            "description": "Semantic search across the codebase with optional relevance scores and preview mode. Returns relevant code snippets with file paths and line numbers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)", "default": 10 },
                    "path": { "type": "string", "description": "Subdirectory to search within (e.g., 'src/api')" },
                    "glob": { "type": "string", "description": "File pattern filter (e.g., '*.rs', '*.{ts,tsx}')" },
                    "include_scores": { "type": "boolean", "description": "Include relevance scores (0.0-1.0)", "default": false },
                    "preview_mode": { "type": "boolean", "description": "Return short preview (2-3 lines) instead of full content. Saves 80% tokens.", "default": false },
                    "min_score": { "type": "number", "description": "Minimum relevance score to include (0.0-1.0, filters low-quality results)", "default": 0.0 },
                    "include_context": { "type": "boolean", "description": "Include context (function/class name where match found)", "default": true }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "get_symbols",
            "description": "List all symbols (functions, structs, classes) in a file or the entire project. Supports pagination via offset/limit. Returns a token-optimized map clustered by file path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Filter by file path (optional)" },
                    "kind": { "type": "string", "description": "Filter by symbol kind: function, struct, class, interface, etc." },
                    "offset": { "type": "integer", "description": "Pagination offset (default: 0)", "default": 0 },
                    "limit": { "type": "integer", "description": "Max results (default: 200, max: 500)", "default": 200 }
                }
            }
        }),
        json!({
            "name": "get_references",
            "description": "Find all references to a symbol (where it's used in the codebase). Returns a token-optimized flat string array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to find references for" }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "get_dependencies",
            "description": "List project dependencies from Cargo.toml/package.json with versions. Returns a token-optimized flat string array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ecosystem": { "type": "string", "description": "Filter by ecosystem: cargo, npm (optional)" }
                }
            }
        }),
        json!({
            "name": "dependency_impact",
            "description": "Show all files that use a specific dependency. Returns a token-optimized flat string array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Dependency name (e.g., 'tokio', 'react')" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "git_blame",
            "description": "Get git blame for a single line or a range. Pass `line` for one line, or `start_line`+`end_line` for a span. Returns one entry per blame hunk with `lines_in_hunk` so the caller can see how far each commit's authorship extends.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path" },
                    "line": { "type": "integer", "description": "Single line (legacy; use start_line/end_line for ranges)" },
                    "start_line": { "type": "integer", "description": "First line of the range to blame (1-based, inclusive)" },
                    "end_line": { "type": "integer", "description": "Last line of the range to blame (1-based, inclusive). Defaults to start_line." }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "git_history",
            "description": "Get recent commit history for a file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path" },
                    "limit": { "type": "integer", "description": "Max commits to return (default: 10)", "default": 10 }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "context_bundle",
            "description": "Build a context bundle for a file, resolving its import dependencies recursively. Use skeleton=true to skeletonize everything, or skeleton_deps_only=true to keep main file full but skeletonize dependencies (saves tokens while preserving target context).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path to bundle context for" },
                    "depth": { "type": "integer", "description": "How deep to resolve dependencies (default: 2)", "default": 2 },
                    "skeleton": { "type": "boolean", "description": "If true, strip function bodies from ALL files (main + deps)", "default": false },
                    "skeleton_deps_only": { "type": "boolean", "description": "If true, keep main file full but skeletonize dependencies only", "default": false }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "search_by_purpose",
            "description": "Search files by high-level purpose/responsibility. Best for architectural queries like 'authentication', 'billing logic', 'API routes'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language description of what you're looking for" },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)", "default": 10 }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "structural_search",
            "description": "Search code by AST shape, not regex. Use `preset` for curated patterns or `query` + `language` for a custom tree-sitter S-expression. Call without args to list all presets. Presets: Rust (rust_unwrap, rust_expect, rust_panic, rust_todo_unimplemented, rust_dbg, rust_println, rust_clone), TS/JS (ts_any, ts_console_log, ts_ts_ignore, ts_debugger, ts_non_null), Python (py_print, py_bare_except, py_breakpoint), Go (go_panic, go_fmt_print). Returns hits with file/line/col + matched text. Much more precise than grep — comments and strings are ignored; matches respect syntax.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "description": "Catalog preset id (see description). Mutually exclusive with `query`." },
                    "query": { "type": "string", "description": "Custom tree-sitter S-expression. Requires `language`. Use `@hit` capture to mark the node returned in results." },
                    "language": { "type": "string", "description": "Target language (rust, typescript, python, go, ...). Required with `query`; optional filter with `preset`." },
                    "path": { "type": "string", "description": "Subdirectory to search in (relative to project root)" },
                    "max_results": { "type": "integer", "description": "Cap on returned hits (default 200, max 2000)", "default": 200 },
                    "include_text": { "type": "boolean", "description": "Include the matched code snippet (truncated to 200 chars). Default: true.", "default": true }
                }
            }
        }),
        json!({
            "name": "complexity",
            "description": "Cyclomatic complexity + size metrics per function (McCabe: 1 + decision points). Counts branches (if/elif, match/switch arms, loops, except/catch), short-circuit operators (&&, ||, ??), and ternaries. Also reports line count, param count, max nesting depth. Use to find refactor candidates and likely bug sites. Ratings: 1-5 simple, 6-10 moderate, 11-20 complex, 21+ very_complex. Nested closures count toward the enclosing fn; nested named functions get their own entry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Analyze a single file (relative path). Mutually exclusive with `path`." },
                    "path": { "type": "string", "description": "Analyze all files under this subdirectory (default: whole project)" },
                    "min_complexity": { "type": "integer", "description": "Only return functions with complexity >= this (default 1 = all)", "default": 1 },
                    "sort": { "type": "string", "enum": ["complexity", "lines", "nesting", "name"], "description": "Sort order (default: complexity desc)", "default": "complexity" },
                    "limit": { "type": "integer", "description": "Max functions to return (default 100, max 1000)", "default": 100 }
                }
            }
        }),
        json!({
            "name": "find_unreachable",
            "description": "Detect statically unreachable code: statements that follow an unconditional terminator (return / break / continue / throw / raise / panic!/unreachable!/todo!/unimplemented!) in the same block. Only direct siblings count — a `return` inside an `if` branch does NOT flag code after the `if` (that's reachable when the condition is false), so false positives are near zero. Out of scope: unreachable match arms after a catch-all, always-false conditions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Analyze a single file (relative path). Mutually exclusive with `path`." },
                    "path": { "type": "string", "description": "Analyze all files under this subdirectory (default: whole project)" },
                    "limit": { "type": "integer", "description": "Max findings to return (default 200, max 2000)", "default": 200 }
                }
            }
        }),
        json!({
            "name": "skeleton",
            "description": "Read file in skeleton mode (signatures only, no function bodies). Saves 3-5× tokens while preserving structure. Shows imports, types, function signatures, and doc comments.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "File path to skeletonize (relative to project root)"
                    },
                    "include_private": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include private/internal items (default: public only)"
                    },
                    "include_tests": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include test functions (default: false)"
                    }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "read_file",
            "description": "Read file content with optional line range. Returns the file text with line numbers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Relative file path" },
                    "start_line": { "type": "integer", "description": "First line to read (1-based, default: 1)", "default": 1 },
                    "end_line": { "type": "integer", "description": "Last line to read (inclusive, default: end of file)" }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "project_tree",
            "description": "Show directory tree of the project. Respects .gitignore and skips common noise directories (node_modules, target, .git, etc.). Returns a token-optimized flat string array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Subdirectory to list (relative, default: project root)", "default": "" },
                    "depth": { "type": "integer", "description": "Max depth to recurse (default: 3)", "default": 3 },
                    "pattern": { "type": "string", "description": "Glob pattern to filter files (e.g., '*.rs', '*.{ts,tsx}')" }
                }
            }
        }),
        json!({
            "name": "search_symbols",
            "description": "Search symbols (functions, structs, classes) by name pattern. Supports substring matching. Returns a token-optimized map clustered by file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Symbol name or substring to search for" },
                    "kind": { "type": "string", "description": "Filter by symbol kind: function, struct, class, interface, etc. (optional)" },
                    "limit": { "type": "integer", "description": "Maximum results (default: 20)", "default": 20 }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "call_path",
            "description": "BFS over symbol_references between two named symbols. `direction=calls` (default): paths where `from` transitively reaches `to` via outgoing calls. `direction=called_by`: paths where `from` is reached by walking incoming references from `to`. Returns shortest paths rendered as `from → ... → to`. Use `file_from`/`file_to` to disambiguate when names collide. Caveats: dyn/trait dispatch isn't tracked; unresolved refs fan out via name lookup (occasional false branches).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source symbol name" },
                    "to": { "type": "string", "description": "Target symbol name" },
                    "direction": {
                        "type": "string",
                        "enum": ["calls", "called_by"],
                        "description": "calls = walk outgoing edges; called_by = walk incoming edges",
                        "default": "calls"
                    },
                    "file_from": { "type": "string", "description": "Disambiguate `from` by file path (optional)" },
                    "file_to": { "type": "string", "description": "Disambiguate `to` by file path (optional)" },
                    "max_depth": { "type": "integer", "description": "BFS depth cap (default 8, max 30)", "default": 8 },
                    "max_paths": { "type": "integer", "description": "Max distinct paths to return (default 5, max 50)", "default": 5 }
                },
                "required": ["from", "to"]
            }
        }),
        json!({
            "name": "dependency_subgraph",
            "description": "Dependency neighbourhood around a symbol via BFS over symbol_references. `direction=out` (default): what the symbol depends on; `in`: what depends on it; `both`: union. Returns nodes + edges within `max_depth`, bounded by `max_nodes`. Unlike call_path (path to a target), this is the whole reachable subgraph — good for impact analysis and understanding a symbol's blast radius. Caveats: dyn/trait dispatch not tracked; only resolved references are followed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Center symbol name" },
                    "file": { "type": "string", "description": "Disambiguate by file path (optional)" },
                    "direction": {
                        "type": "string",
                        "enum": ["out", "in", "both"],
                        "description": "out = dependencies; in = dependents; both = union",
                        "default": "out"
                    },
                    "max_depth": { "type": "integer", "description": "BFS depth (default 3, max 20)", "default": 3 },
                    "max_nodes": { "type": "integer", "description": "Node cap (default 50, max 500)", "default": 50 }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "find_implementations",
            "description": "Find implementations of a trait / interface / base class by name, across languages. Rust: `impl <Name> for <Type>` blocks (the trait position only — the implementing type doesn't false-match). TS/JS: `class X implements <Name>` and `class X extends <Name>`. Python: `class X(<Name>)` base classes. Whole-token matching (`Foo` won't match `FooBar`). Go is NOT supported — interface satisfaction is structural (method sets), not declared. Needs re-indexed signatures.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Trait / interface / base class name to find implementors of" },
                    "limit": { "type": "integer", "description": "Max results (default 100, max 500)", "default": 100 }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "find_unused_imports",
            "description": "Find imports in a file whose local binding is never used in the rest of the file. Parses imports via tree-sitter, then word-boundary matches each local binding against non-import lines. Skips wildcards (`use foo::*`, `from foo import *`) and `pub use` re-exports (use `include_reexports=true` to include them). Caveats: false positives on macros only referenced by name through `paste!` / `concat_idents!`; false negatives if a binding has the same name as a method called on an unrelated type.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path relative to project root" },
                    "include_reexports": {
                        "type": "boolean",
                        "description": "Include `pub use` (Rust) — by default re-exports are skipped because they may be consumed from outside the file.",
                        "default": false
                    }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "find_by_type_signature",
            "description": "Find functions/methods by type signature — region-aware. `returns` matches the return-type region only (so it won't false-match params), `param_type` matches the parameter region only, `signature_contains` is a raw substring over the whole signature. Case-sensitive substrings, so `Result<MyType` matches `Result<MyType, Error>` and `&mut Conn` matches `&mut Connection`. At least one filter required. Caveats: Go method receivers count as a param; Rust where-clauses bleed into the return region; needs re-indexed signatures.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "returns": { "type": "string", "description": "Substring the return type must contain (e.g. 'Result<', 'Promise<void>', 'error')" },
                    "param_type": { "type": "string", "description": "Substring a parameter type must contain (e.g. '&mut Connection', ': number')" },
                    "signature_contains": { "type": "string", "description": "Raw substring anywhere in the signature (fallback for generics/lifetimes/where-clauses)" },
                    "kind": { "type": "string", "description": "Restrict to one kind (function/method). Default: both." },
                    "file": { "type": "string", "description": "Restrict to files whose path contains this substring" },
                    "limit": { "type": "integer", "description": "Max results (default 100, max 500)", "default": 100 }
                }
            }
        }),
        json!({
            "name": "find_unused_symbols",
            "description": "Find symbols with no incoming `call`/`usage`/`inherit`/`type_usage` references — dead-code candidates. Walks the symbol_references graph (both resolved by id and unresolved by name) and applies cleanup heuristics: tests/benches by path AND by attribute, entry points by name, FFI/wasm/Python exports by signature attributes. Caveats: the graph is language-agnostic. Attribute/decorator filtering works for Rust (#[test], #[wasm_bindgen]), Python (@pytest.fixture) and TS (@Component) — decorators are now captured into the signature. Go has no attribute markers (path+naming only). External consumers of public API aren't visible (use `public_only=false`); trait/dyn dispatch isn't tracked (false positives on trait impl methods); attribute filtering requires re-indexed data — old indexes need `force_reindex`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Restrict to one symbol kind. Default: all meaningful kinds (function, method, struct, enum, trait, interface, class, const, type, type_alias)." },
                    "file": { "type": "string", "description": "Restrict to files whose path contains this substring (relative to project root)" },
                    "public_only": { "type": "boolean", "description": "Only consider symbols whose signature starts with `pub`/`export` (or whose name doesn't start with `_`). Default: true.", "default": true },
                    "exclude_tests": { "type": "boolean", "description": "Skip test files by path (tests/, test/, __tests__/, benches/, *.test.*, *.spec.*, *_test.*) AND by attribute in signature (#[test], #[tokio::test], #[bench], #[cfg(test)], @pytest.fixture, @pytest.mark) AND by naming convention (test_*, *_test). Default: true.", "default": true },
                    "exclude_entry_points": { "type": "boolean", "description": "Skip symbols named `main`, `__main__`, `lambda_handler`, `handler` — typical CLI/Lambda entry points. Default: true.", "default": true },
                    "exclude_exports": { "type": "boolean", "description": "Skip symbols whose signature contains FFI/wasm/Python/Node export markers: #[no_mangle], extern \"C\", #[wasm_bindgen], #[pyfunction], #[napi], @customElement, @Component, #[export_name]. Default: true.", "default": true },
                    "limit": { "type": "integer", "description": "Max symbols to return (default 100, max 500)", "default": 100 }
                }
            }
        }),
        json!({
            "name": "grep",
            "description": "Search file contents using regex patterns. Returns a token-optimized map of matching lines clustered by file path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": { "type": "string", "description": "Subdirectory to search in (relative to project root)" },
                    "glob": { "type": "string", "description": "File filter glob — only simple `*.<ext>` is supported" },
                    "case_insensitive": { "type": "boolean", "description": "Case-insensitive search (default: false)" },
                    "context_lines": { "type": "integer", "description": "Number of context lines before/after match (default: 0)", "default": 0 },
                    "max_results": { "type": "integer", "description": "Max matches (default: 100)", "default": 100 }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "find_files",
            "description": "Find files by glob pattern. Respects .gitignore. Returns matching file paths along with `total`, `count`, `truncated`, `limit`, and `offset` so callers can detect when the list was capped.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern (e.g., '*.rs', '**/*.tsx', 'Cargo.*')" },
                    "path": { "type": "string", "description": "Subdirectory to search in (relative to project root)" },
                    "limit": { "type": "integer", "description": "Max files to return (default 100, max 10000)", "default": 100 },
                    "offset": { "type": "integer", "description": "Number of files to skip (for pagination)", "default": 0 }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "git_diff",
            "description": "Show git diff for staged or unstaged changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path (relative) to show diff for a specific file" },
                    "staged": { "type": "boolean", "description": "Show staged changes instead of unstaged (default: false)", "default": false }
                }
            }
        }),
        json!({
            "name": "get_callers",
            "description": "Find all symbols that call/reference a given symbol (incoming references). Returns a token-optimized flat string array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to find callers for" }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "get_callees",
            "description": "Find all symbols called/referenced by a given symbol (outgoing references). Returns a token-optimized flat string array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to find callees for" },
                    "file": { "type": "string", "description": "File path to disambiguate symbol (optional)" }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "health_check",
            "description": "Check the health status of all gofer components: database, vector store, embedder. Returns detailed status for each component.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        // Phase 0: Index Quality & Visibility
        json!({
            "name": "get_index_status",
            "description": "Get current index status with completeness metrics, file counts, and last sync information. Returns token-optimized status summaries.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "force_reindex",
            "description": "Force reindex of file(s) with priority. Useful when index is stale or incomplete. Supports file, directory, or full project scope.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["file", "directory", "project"],
                        "description": "Scope of reindexing: single file, directory, or entire project",
                        "default": "file"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory path (required for file/directory scope)"
                    }
                }
            }
        }),
        // Phase 0: Lightweight Checks (Token Efficient)
        json!({
            "name": "suggest_commit",
            "description": "Generate intelligent commit message based on git changes. Analyzes diff and suggests Conventional Commits format with safety checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "style": {
                        "type": "string",
                        "enum": ["conventional", "simple", "detailed"],
                        "default": "conventional",
                        "description": "Commit message style"
                    },
                    "include_emoji": {
                        "type": "boolean",
                        "default": true,
                        "description": "Add emoji to subject line (✨ feat, 🐛 fix, etc.)"
                    },
                    "max_subject_length": {
                        "type": "integer",
                        "default": 72,
                        "description": "Maximum subject line length"
                    }
                }
            }
        }),
        json!({
            "name": "read_function_context",
            "description": "Extract a single function with its dependencies (imports, types, called functions). Saves 90-95% tokens vs read_file by providing only relevant context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "function": {
                        "type": "string",
                        "description": "Function name to extract"
                    },
                    "include_types": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include type definitions referenced by the function"
                    },
                    "include_imports": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include used import statements"
                    },
                    "include_callees": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include functions called by this function (1 level deep)"
                    }
                },
                "required": ["file", "function"]
            }
        }),
        json!({
            "name": "read_types_only",
            "description": "Extract only type definitions from a file (structs, enums, interfaces, type aliases, traits). Saves 90-95% tokens when analyzing data models.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["struct", "enum", "interface", "type_alias", "trait", "class"],
                        "description": "Filter by specific type kind (optional)"
                    },
                    "include_docs": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include doc comments"
                    }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "smart_file_selection",
            "description": "Get a ranked list of files relevant to a task or question. Helps AI choose which files to read by combining vector search, symbol matching, and path analysis.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language description of task or question"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 5,
                        "description": "Number of files to return (default: 5)"
                    },
                    "min_score": {
                        "type": "number",
                        "default": 0.3,
                        "description": "Minimum relevance score 0-1 (default: 0.3)"
                    },
                    "boost_recency": {
                        "type": "number",
                        "default": 0.2,
                        "description": "How much recency affects ranking (0.0 = ignore recency, 0.2 = small tiebreaker, 1.0 = legacy behaviour)"
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "batch_operations",
            "description": "Execute multiple read/search operations in a single request. Reduces latency by 3-5× through parallel execution and reduced network overhead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "description": "List of operations to execute",
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": ["read_file", "get_symbols", "search", "skeleton"],
                                    "description": "Operation type"
                                },
                                "params": {
                                    "type": "object",
                                    "description": "Parameters for the operation"
                                }
                            },
                            "required": ["type", "params"]
                        }
                    },
                    "parallel": {
                        "type": "boolean",
                        "default": true,
                        "description": "Execute operations in parallel (default: true)"
                    },
                    "continue_on_error": {
                        "type": "boolean",
                        "default": true,
                        "description": "Continue if one operation fails (default: true)"
                    },
                    "summary_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Drop the per-operation `data` payload — returns just success/error/timing. Useful when you only need to know which operations succeeded."
                    },
                    "max_chars_per_op": {
                        "type": "integer",
                        "description": "Truncate each operation's serialized data to this many characters. Adds `data_truncated: true` and `data_full_chars` so callers know the full size."
                    }
                },
                "required": ["operations"]
            }
        }),
        // Phase 1: File Operations
        json!({
            "name": "list_directory",
            "description": "List directory contents with recursive support. Returns a token-optimized flat string array of paths and sizes. Supports exclude patterns for node_modules, target, etc.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (relative to project root)",
                        "default": "."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Recursively list subdirectories",
                        "default": false
                    },
                    "exclude_patterns": {
                        "type": "array",
                        "description": "Patterns to exclude (default: node_modules, target, .git, dist, build)",
                        "items": { "type": "string" }
                    }
                }
            }
        }),
        json!({
            "name": "get_file_metadata",
            "description": "Get file metadata: size, modification time, line count, binary detection. Use before reading large files to decide on reading strategy.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    }
                },
                "required": ["path"]
            }
        }),
        // Project stats & Vue
        json!({
            "name": "domain_stats",
            "description": "Show symbol count breakdown by domain/directory. Returns a map of domain paths to their symbol counts.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_vue_tree",
            "description": "Get the Vue component tree for a .vue file. Returns the parent-child component relationships extracted from the index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Path to the .vue file (relative to project root)" }
                },
                "required": ["file"]
            }
        }),
        // Lightweight existence checks
        json!({
            "name": "file_exists",
            "description": "Check whether a file exists in the project. Cheaper than read_file for existence-only checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path relative to project root" }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "symbol_exists",
            "description": "Check whether a named symbol exists in the index. Optionally scoped to a single file for disambiguation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to look up" },
                    "file": { "type": "string", "description": "Restrict check to this file path (optional)" }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "is_exported",
            "description": "Check whether a symbol is exported/public. Uses signature heuristics (pub, export keywords) to determine visibility.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to check" },
                    "file": { "type": "string", "description": "File path to disambiguate when the symbol name is not unique (optional)" }
                },
                "required": ["symbol"]
            }
        }),
        // Diagnostics
        json!({
            "name": "has_tests_for",
            "description": "Check whether a test file exists for a given source file. Looks for common naming conventions (.test.ts, .spec.ts, _test.rs, test_*.py, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Source file path relative to project root" }
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "get_config_keys",
            "description": "List all configuration keys with their data types, sources, and required status.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        // Index quality
        json!({
            "name": "validate_index",
            "description": "Validate index integrity: detect files missing symbols, orphaned data, failed indexing, broken references, and embedding gaps. Returns issues with severity and remediation recommendations.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_cache_stats",
            "description": "Get in-memory cache statistics: hit/miss counts, evictions, and current cache size.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_query_stats",
            "description": "Get database query performance metrics: total queries, slow query count and rate, and average query time.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}
