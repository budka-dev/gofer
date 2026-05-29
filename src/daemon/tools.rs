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
        // "get_errors" => diagnostics::tool_get_errors(args, ctx).await,
        "run_diagnostics" => diagnostics::tool_run_diagnostics(args, ctx).await,
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
        "verify_patch" => git::tool_verify_patch(args, ctx).await,
        "read_file" => files::tool_read_file(args, ctx).await,
        "project_tree" => project::tool_project_tree(args, ctx).await,
        "search_symbols" => symbols::tool_search_symbols(args, ctx).await,
        "add_rule" => project::tool_add_rule(args, ctx).await,
        "mark_golden_sample" => project::tool_mark_golden_sample(args, ctx).await,
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
        "patch_file" => file_ops::tool_patch_file(args, ctx).await,
        "write_file" => file_ops::tool_write_file(args, ctx).await,
        "append_to_file" => file_ops::tool_append_to_file(args, ctx).await,
        "create_directory" => file_ops::tool_create_directory(args, ctx).await,
        "move_file" => file_ops::tool_move_file(args, ctx).await,
        // Trash management
        "delete_safe" => trash::tool_delete_safe(args, ctx).await,
        "list_trash" => trash::tool_list_trash(args, ctx).await,
        "restore" => trash::tool_restore(args, ctx).await,
        "purge_trash" => trash::tool_purge_trash(args, ctx).await,
        // Atomic Transactions (Phase 2) — multi-file operations with auto-rollback
        "begin_transaction" => transactions::tool_begin_transaction(args, ctx).await,
        "add_operation" => transactions::tool_add_operation(args, ctx).await,
        "commit_transaction" => transactions::tool_commit_transaction(args, ctx).await,
        "rollback_transaction" => transactions::tool_rollback_transaction(args, ctx).await,
        "list_transactions" => transactions::tool_list_transactions(args, ctx).await,
        // Code Quality Tools (Phase 2)
        "format_file" => code_quality::tool_format_file(args, ctx).await,
        "lint_file" => code_quality::tool_lint_file(args, ctx).await,
        "apply_lint_fix" => code_quality::tool_apply_lint_fix(args, ctx).await,
        // CAS Buffer (Phase 3) - content-addressable storage
        "clipboard_copy" => cas_buffer::tool_extract_to_clipboard(args, ctx).await,
        "clipboard_paste" => cas_buffer::tool_insert_clipboard(args, ctx).await,
        "clipboard_replace" => cas_buffer::tool_replace_with_clipboard(args, ctx).await,
        "clipboard_store_text" => cas_buffer::tool_content_to_clipboard(args, ctx).await,
        "clipboard_list" => cas_buffer::tool_list_clipboards(args, ctx).await,
        "clipboard_clear" => cas_buffer::tool_clear_clipboard(args, ctx).await,
        // Execution Sandbox (Phase 3) - AI can test its own code
        "execute_code" => sandbox::tool_execute_code(args, ctx).await,
        "execute_function" => sandbox::tool_execute_function(args, ctx).await,
        "run_test" => sandbox::tool_run_test(args, ctx).await,
        "run_all_tests" => sandbox::tool_run_all_tests(args, ctx).await,
        // lsp tools
        "lsp_goto_definition" => lsp::tool_lsp_goto_definition(args, ctx).await,
        "lsp_find_references" => lsp::tool_lsp_find_references(args, ctx).await,
        "lsp_hover" => lsp::tool_lsp_hover(args, ctx).await,
        "lsp_diagnostics" => lsp::tool_lsp_diagnostics(args, ctx).await,
        "lsp_completions" => lsp::tool_lsp_completions(args, ctx).await,
        "lsp_inlay_hints" => lsp::tool_lsp_inlay_hints(args, ctx).await,
        "lsp_code_actions" => lsp::tool_lsp_code_actions(args, ctx).await,
        // lsp extended (architecture navigation)
        "lsp_document_symbols" => lsp::tool_lsp_document_symbols(args, ctx).await,
        "lsp_workspace_symbols" => lsp::tool_lsp_workspace_symbols(args, ctx).await,
        "lsp_goto_implementation" => lsp::tool_lsp_goto_implementation(args, ctx).await,
        "lsp_rename" => lsp::tool_lsp_rename(args, ctx).await,
        "lsp_expand_macro" => lsp::tool_lsp_expand_macro(args, ctx).await,
        "lsp_incoming_calls" => lsp::tool_lsp_incoming_calls(args, ctx).await,
        "lsp_outgoing_calls" => lsp::tool_lsp_outgoing_calls(args, ctx).await,
        // Language tools folding (meta-tools)
        "lang_tools_list" => lang_tools::tool_lang_tools_list(args, ctx).await,
        "lang_tools_call" => lang_tools::tool_lang_tools_call(args, ctx).await,
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
        // json!({
        //     "name": "get_errors",
        //     "description": "Get current compiler errors/warnings from cargo check or tsc. Supports pagination via offset/limit. Returns a token-optimized map clustered by file.",
        //     "inputSchema": {
        //         "type": "object",
        //         "properties": {
        //             "file": { "type": "string", "description": "Filter errors by file path (optional)" },
        //             "severity": { "type": "string", "description": "Filter by severity: error, warning (optional)" },
        //             "offset": { "type": "integer", "description": "Pagination offset (default: 0)", "default": 0 },
        //             "limit": { "type": "integer", "description": "Max results (default: 200, max: 500)", "default": 200 }
        //         }
        //     }
        // }),
        json!({
            "name": "run_diagnostics",
            "description": "Run cargo check and/or tsc to refresh compiler diagnostics. You can pass options for cargo check to target specific workspaces, packages, or all targets. Pass `file` to filter the diagnostics returned to a single file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": { "type": "boolean", "description": "Check all packages in the workspace (cargo check --workspace)" },
                    "all_targets": { "type": "boolean", "description": "Check all targets (cargo check --all-targets) including tests and benches" },
                    "package": { "type": "string", "description": "Package to check (cargo check -p <package>)" },
                    "manifest_path": { "type": "string", "description": "Path to Cargo.toml (cargo check --manifest-path <path>)" },
                    "file": { "type": "string", "description": "Only return diagnostics whose file path contains this string (relative to project root)" }
                }
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
            "name": "verify_patch",
            "description": "Verify a code patch by temporarily applying it and running the compiler/linter. Returns diagnostics (errors, warnings) without modifying the file permanently.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Relative file path to verify (e.g. src/main.rs)" },
                    "content": { "type": "string", "description": "Full file content to verify (the patched version)" }
                },
                "required": ["file", "content"]
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
        json!({
            "name": "patch_file",
            "description": "Precise search & replace in files. Token-efficient: only specify changed code, not entire file. Supports replacing specific occurrence or all occurrences.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "search_string": {
                        "type": "string",
                        "description": "Exact text to find"
                    },
                    "replace_string": {
                        "type": "string",
                        "description": "Replacement text"
                    },
                    "occurrence": {
                        "type": "integer",
                        "description": "Which occurrence to replace (1-indexed, 0 = all)",
                        "default": 1
                    }
                },
                "required": ["path", "search_string", "replace_string"]
            }
        }),
        json!({
            "name": "write_file",
            "description": "Create new file or overwrite existing. Use for new files; prefer patch_file for modifications to avoid regenerating entire file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "content": {
                        "type": "string",
                        "description": "File content"
                    },
                    "create_dirs": {
                        "type": "boolean",
                        "description": "Create parent directories if needed (like mkdir -p)",
                        "default": false
                    }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "append_to_file",
            "description": "Append content to end of file. Safe for adding new functions, environment variables, or log entries without modifying existing content.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to append"
                    },
                    "newline_before": {
                        "type": "boolean",
                        "description": "Add newline before content if file doesn't end with one",
                        "default": true
                    }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "create_directory",
            "description": "Create directory with optional recursive creation of parent directories.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (relative to project root)"
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Create parent directories (like mkdir -p)",
                        "default": true
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "move_file",
            "description": "Move or rename file/directory. Fails if destination exists unless overwrite=true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source path (relative to project root)"
                    },
                    "destination": {
                        "type": "string",
                        "description": "Destination path (relative to project root)"
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "Overwrite if destination exists",
                        "default": false
                    }
                },
                "required": ["source", "destination"]
            }
        }),
        // Atomic Transactions (Phase 2) — multi-file operations with auto-rollback
        json!({
            "name": "begin_transaction",
            "description": "Open a new transaction. Subsequent add_operation/commit_transaction/rollback_transaction calls reference its `transaction_id`. State lives in the daemon process and is lost on restart.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": { "type": "string", "description": "Optional id. If omitted, a UUID is generated." }
                }
            }
        }),
        json!({
            "name": "add_operation",
            "description": "Stage an operation in an open transaction. Supported `operation.type`: patch_file, write_file, append_to_file, delete_safe, move_file, create_directory. Each staged operation is validated (basic syntax + conflicts).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": { "type": "string", "description": "Id from begin_transaction" },
                    "operation": {
                        "type": "object",
                        "description": "{type: <op>, params: {...}}",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["patch_file", "write_file", "append_to_file", "delete_safe", "move_file", "create_directory"]
                            },
                            "params": { "type": "object", "description": "Same arguments shape as the corresponding standalone tool" }
                        },
                        "required": ["type", "params"]
                    }
                },
                "required": ["transaction_id", "operation"]
            }
        }),
        json!({
            "name": "commit_transaction",
            "description": "Snapshot all affected files, apply staged operations one by one. On any failure, all already-applied operations are rolled back from snapshots and the transaction is marked failed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": { "type": "string", "description": "Id from begin_transaction" }
                },
                "required": ["transaction_id"]
            }
        }),
        json!({
            "name": "rollback_transaction",
            "description": "Discard all staged operations and mark the transaction rolled_back. No-op if already committed/failed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": { "type": "string", "description": "Id from begin_transaction" }
                },
                "required": ["transaction_id"]
            }
        }),
        json!({
            "name": "list_transactions",
            "description": "List all transactions held in process memory with their status and operations count.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        // Trash management (safe deletion with recovery)
        json!({
            "name": "delete_safe",
            "description": "Safely delete file/directory by moving to trash. Can be restored later. Stores metadata (reason, tags) for context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to delete (relative to project root)"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for deletion (optional, for context)"
                    },
                    "tags": {
                        "type": "array",
                        "description": "Tags for categorization (e.g., ['refactor', 'deprecated'])",
                        "items": { "type": "string" }
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "list_trash",
            "description": "Show trash contents with metadata. Returns deletion history, sizes, and restore information.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "restore",
            "description": "Restore file/directory from trash to original location or specified path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "deletion_uuid": {
                        "type": "string",
                        "description": "UUID from delete_safe operation"
                    },
                    "target_path": {
                        "type": "string",
                        "description": "Alternative restore path (optional)"
                    }
                },
                "required": ["deletion_uuid"]
            }
        }),
        json!({
            "name": "purge_trash",
            "description": "Permanently delete from trash. Optionally specify deletion_uuid to purge specific item, or omit to purge all.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "deletion_uuid": {
                        "type": "string",
                        "description": "UUID to purge (optional, omit to purge all)"
                    }
                }
            }
        }),
        // Atomic Transactions (Phase 2) - safe multi-file operations
        // Code Quality Tools (Phase 2) - formatters and linters
        json!({
            "name": "format_file",
            "description": "Auto-format file using appropriate formatter (rustfmt, prettier, black, gofmt). Detects formatter by extension. Returns diff info.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "formatter": {
                        "type": "string",
                        "description": "Override formatter (optional, auto-detected by extension)",
                        "enum": ["rustfmt", "prettier", "black", "gofmt"]
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "lint_file",
            "description": "Run linter on file (clippy, eslint, ruff, golangci-lint). Returns token-optimized flat string array of warnings with line numbers, severity, and auto-fix availability.",
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
        json!({
            "name": "apply_lint_fix",
            "description": "Apply automatic fixes from linter (clippy --fix, eslint --fix, ruff --fix). Only fixes auto-fixable issues.",
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
        // CAS Buffer (Phase 3) - revolutionary token optimization
        json!({
            "name": "clipboard_copy",
            "description": "Extract code block to content-addressable hash. Returns short hash ID instead of full content. Saves 70-90% tokens. Optionally cut (remove) from source file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Start line (1-indexed)"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "End line (1-indexed, inclusive)"
                    },
                    "cut": {
                        "type": "boolean",
                        "description": "Remove block from source file (default: false = copy)",
                        "default": false
                    }
                },
                "required": ["path", "start_line", "end_line"]
            }
        }),
        json!({
            "name": "clipboard_paste",
            "description": "Insert code from hash at specified line. No need to regenerate code - server expands hash to original content. Zero risk of hallucinations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "line_number": {
                        "type": "integer",
                        "description": "Line to insert at (1-indexed, 0 = beginning)"
                    },
                    "hash_id": {
                        "type": "string",
                        "description": "Hash ID from extract_to_hash or content_to_hash"
                    }
                },
                "required": ["path", "line_number", "hash_id"]
            }
        }),
        json!({
            "name": "clipboard_replace",
            "description": "Replace code block with content from hash. Precise replacement without regenerating code.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Start line to replace (1-indexed)"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "End line to replace (1-indexed, inclusive)"
                    },
                    "hash_id": {
                        "type": "string",
                        "description": "Hash ID from extract_to_hash or content_to_hash"
                    }
                },
                "required": ["path", "start_line", "end_line", "hash_id"]
            }
        }),
        json!({
            "name": "clipboard_store_text",
            "description": "Create hash from arbitrary content. Useful when AI generates code and wants to store it as hash for later reuse.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Code content to store"
                    }
                },
                "required": ["content"]
            }
        }),
        json!({
            "name": "clipboard_list",
            "description": "Show all active hashes in memory with metadata (size, age, access count, TTL). Use to see what's available.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "clipboard_clear",
            "description": "Remove hash from memory. Optionally specify hash_id to clear specific buffer, or omit to clear all buffers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "hash_id": {
                        "type": "string",
                        "description": "Hash ID to clear (optional, omit to clear all)"
                    }
                }
            }
        }),
        // Execution Sandbox (Phase 3) - AI becomes engineer, not just generator
        json!({
            "name": "execute_code",
            "description": "Execute arbitrary code snippet in isolated environment. Returns stdout/stderr and execution result. AI can test code before committing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Code to execute"
                    },
                    "language": {
                        "type": "string",
                        "enum": ["rust", "python", "javascript", "js"],
                        "description": "Programming language"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 5, max: 60)",
                        "default": 5
                    }
                },
                "required": ["code", "language"]
            }
        }),
        json!({
            "name": "execute_function",
            "description": "Execute specific function from file with arguments. Returns function result or error. Perfect for testing individual functions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "function_name": {
                        "type": "string",
                        "description": "Function name to execute"
                    },
                    "args": {
                        "type": "array",
                        "description": "Function arguments as JSON array",
                        "items": {}
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 5, max: 60)",
                        "default": 5
                    }
                },
                "required": ["path", "function_name"]
            }
        }),
        json!({
            "name": "run_test",
            "description": "Run specific test or all tests in file. Returns pass/fail status with details. AI can verify code correctness immediately.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Test file path (relative to project root)"
                    },
                    "test_name": {
                        "type": "string",
                        "description": "Specific test name (optional, omit to run all tests in file)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 30, max: 60)",
                        "default": 30
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "run_all_tests",
            "description": "Run entire project test suite. Auto-detects test framework (cargo test, npm test, pytest). Returns summary with pass/fail counts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Test filter/pattern (optional)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 60, max: 120)",
                        "default": 60
                    }
                }
            }
        }),
        //LSP client tools
        json!({
            "name": "lsp_goto_definition",
            "description": "Go to definition for a Rust symbol at the specified position using LSP client. Returns precise location(s) of where the symbol is defined.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_find_references",
            "description": "Find all references to a Rust symbol at the specified position using LSP client. Shows where the symbol is used across the codebase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    },
                    "include_declaration": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include the symbol declaration in results"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_hover",
            "description": "Get hover information (type signature, documentation) for a Rust symbol at the specified position using LSP client.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_diagnostics",
            "description": "Get compiler diagnostics (errors, warnings) for a file.\nReturns a token-optimized flat string array: ['line:char-end:char [severity] code message (source)'] from LSP client. Real-time error checking without running cargo.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    }
                },
                "required": ["file_path"]
            }
        }),
        json!({
            "name": "lsp_completions",
            "description": "Get code completions for Rust at position.\nReturns a token-optimized flat string array: ['label (kind) - detail'] for Rust at the specified position using LSP client. Provides context-aware completions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_inlay_hints",
            "description": "Get inlay hints (type annotations, parameter names) for a file range.\nReturns a token-optimized flat string array: ['line:char [kind] label'] using LSP client. Shows implicit information inline.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Start line number (0-indexed)"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "End line number (0-indexed)"
                    }
                },
                "required": ["file_path", "start_line", "end_line"]
            }
        }),
        json!({
            "name": "lsp_code_actions",
            "description": "Get code actions (quick fixes, refactorings) for a file range.\nReturns a token-optimized flat string array of available actions. using LSP client. Suggests automated fixes and improvements.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Start line number (0-indexed)"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "End line number (0-indexed)"
                    }
                },
                "required": ["file_path", "start_line", "end_line"]
            }
        }),
        //LSP client extended tools
        json!({
            "name": "lsp_document_symbols",
            "description": "Get document outline (structures, functions, enums, traits, impl blocks) for a file. Returns hierarchical symbol tree for quick navigation without reading entire file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    }
                },
                "required": ["file_path"]
            }
        }),
        json!({
            "name": "lsp_workspace_symbols",
            "description": "Search for symbols (structs, functions, traits, etc.) across the entire workspace by name. Like Ctrl+T in IDEs - finds definitions without knowing file location.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Symbol name or pattern to search for (e.g., 'User', 'handle_', 'Config')"
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "lsp_goto_implementation",
            "description": "Go to concrete implementation(s) of a trait method or type. Critical for Rust - shows actual code that executes, not just trait definition.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_rename",
            "description": "Rename a symbol semantically across the entire workspace. Safe refactoring that updates all references, handles shadowing correctly. Returns workspace edit with all affected files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    },
                    "new_name": {
                        "type": "string",
                        "description": "New name for the symbol"
                    }
                },
                "required": ["file_path", "line", "character", "new_name"]
            }
        }),
        json!({
            "name": "lsp_expand_macro",
            "description": "Expand Rust macro at position to see generated code. CRITICAL for understanding derive macros (Serialize, Debug), procedural macros (sqlx::query!, tokio::main), and declarative macros.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number where macro is invoked (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_incoming_calls",
            "description": "Get incoming calls (callers) for a function/method. Shows who calls this function - useful for impact analysis when refactoring.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number of function/method (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lsp_outgoing_calls",
            "description": "Get outgoing calls (callees) for a function/method. Shows what this function calls - useful for understanding dependencies and control flow.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to file (relative or absolute)"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Line number of function/method (0-indexed)"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Character position in line (0-indexed)"
                    }
                },
                "required": ["file_path", "line", "character"]
            }
        }),
        json!({
            "name": "lang_tools_list",
            "description": "List all available language-specific tools (Vue, Rust, TypeScript, etc.). Supports filtering by language and semantic search. Returns tool names, descriptions, and optionally full schemas. Use this to discover what language tools are available before calling them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "lang": {
                        "type": "string",
                        "description": "Optional: Filter by language name (e.g., 'vue', 'rust', 'typescript')"
                    },
                    "search": {
                        "type": "string",
                        "description": "Optional: Semantic search query to find relevant tools by description (e.g., 'component props', 'find references')"
                    },
                    "include_schema": {
                        "type": "boolean",
                        "description": "Optional: Include full inputSchema for each tool (default: false for token efficiency)",
                        "default": false
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "lang_tools_call",
            "description": "Execute a specific language tool by name. Use lang_tools_list first to discover available tools. Provides access to all language-specific functionality (Vue component analysis, Rust LSP features, TypeScript navigation, etc.) without polluting the main tools list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool": {
                        "type": "string",
                        "description": "Tool name to execute (e.g., 'vue_get_meta', 'rust_goto_definition', 'typescript_find_usages')"
                    },
                    "args": {
                        "type": "object",
                        "description": "Arguments to pass to the tool (varies by tool - use lang_tools_list with include_schema=true to see required parameters)"
                    }
                },
                "required": ["tool", "args"]
            }
        }),
    ]
}
