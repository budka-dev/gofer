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
        "context_bundle" => files::tool_context_bundle(args, ctx).await,
        "skeleton" => files::tool_skeleton(args, ctx).await,
        "read_file" => files::tool_read_file(args, ctx).await,
        "project_tree" => project::tool_project_tree(args, ctx).await,
        "search_symbols" => symbols::tool_search_symbols(args, ctx).await,
        "grep" => files::tool_grep(args, ctx).await,
        "find_files" => files::tool_find_files(args, ctx).await,
        "get_callers" => symbols::tool_get_callers(args, ctx).await,
        "get_callees" => symbols::tool_get_callees(args, ctx).await,
        "get_index_status" => index::tool_get_index_status(ctx).await,
        "validate_index" => index::tool_validate_index(ctx).await,
        "file_exists" => files::tool_file_exists(args, ctx).await,
        "symbol_exists" => symbols::tool_symbol_exists(args, ctx).await,
        "find_by_type_signature" => symbols::tool_find_by_type_signature(args, ctx).await,
        "find_implementations" => symbols::tool_find_implementations(args, ctx).await,
        "read_function_context" => files::tool_read_function_context(args, ctx).await,
        "read_types_only" => files::tool_read_types_only(args, ctx).await,
        "batch_operations" => batch::tool_batch_operations(args, ctx).await,
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
            "name": "get_index_status",
            "description": "Get current index status with completeness metrics, file counts, and last sync information. Returns token-optimized status summaries.",
            "inputSchema": { "type": "object", "properties": {} }
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
            "name": "validate_index",
            "description": "Validate index integrity: detect files missing symbols, orphaned data, failed indexing, broken references, and embedding gaps. Returns issues with severity and remediation recommendations.",
            "inputSchema": { "type": "object", "properties": {} }
        })
    ]
}
