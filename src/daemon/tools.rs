//! Core MCP tools: index search + compact read + index ops.
//! Not a replacement for host-agent FS/grep/git tools.

use anyhow::Result;
use serde_json::{json, Value};

use super::handlers::*;
use crate::error::GoferError;

pub use super::handlers::common::ToolContext;

/// Uniform MCP tool payload wrapped as JSON text in `tools/call` content.
///
/// Success:
/// ```json
/// { "ok": true, "tool": "search", "result": { ... }, "meta": { "latency_ms": 12 } }
/// ```
/// Error:
/// ```json
/// { "ok": false, "tool": "search", "error": { "message": "..." }, "meta": { "latency_ms": 3 } }
/// ```
pub fn envelope_ok(tool: &str, result: Value, latency_ms: u64) -> Value {
    json!({
        "ok": true,
        "tool": tool,
        "result": result,
        "meta": { "latency_ms": latency_ms }
    })
}

/// See [`envelope_ok`].
pub fn envelope_err(tool: &str, message: impl AsRef<str>, latency_ms: u64) -> Value {
    json!({
        "ok": false,
        "tool": tool,
        "error": { "message": message.as_ref() },
        "meta": { "latency_ms": latency_ms }
    })
}

#[cfg(test)]
mod envelope_tests {
    use super::*;

    #[test]
    fn envelope_ok_shape() {
        let v = envelope_ok("search", json!({"total": 1}), 5);
        assert_eq!(v["ok"], true);
        assert_eq!(v["tool"], "search");
        assert_eq!(v["result"]["total"], 1);
        assert_eq!(v["meta"]["latency_ms"], 5);
    }

    #[test]
    fn envelope_err_shape() {
        let v = envelope_err("reindex", "path required", 2);
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["message"], "path required");
        assert_eq!(v["meta"]["latency_ms"], 2);
    }
}

/// Dispatch a tool call by name. Returns structured JSON.
pub async fn dispatch(name: &str, args: Value, ctx: &ToolContext) -> Result<Value> {
    match name {
        "search" => search::tool_search(args, ctx).await,
        "get_symbols" => symbols::tool_get_symbols(args, ctx).await,
        "get_references" => symbols::tool_get_references(args, ctx).await,
        "context_bundle" => files::tool_context_bundle(args, ctx).await,
        "skeleton" => files::tool_skeleton(args, ctx).await,
        "search_symbols" => symbols::tool_search_symbols(args, ctx).await,
        "get_callers" => symbols::tool_get_callers(args, ctx).await,
        "get_callees" => symbols::tool_get_callees(args, ctx).await,
        "get_index_status" => index::tool_get_index_status(ctx).await,
        "validate_index" => index::tool_validate_index(ctx).await,
        "reindex" => index::tool_reindex(args, ctx).await,
        "find_by_type_signature" => symbols::tool_find_by_type_signature(args, ctx).await,
        "find_implementations" => symbols::tool_find_implementations(args, ctx).await,
        "read_function_context" => files::tool_read_function_context(args, ctx).await,
        "read_types_only" => files::tool_read_types_only(args, ctx).await,
        "batch_operations" => batch::tool_batch_operations(args, ctx).await,
        _ => Err(GoferError::MethodNotFound(name.to_string()).into()),
    }
}

/// Static list for tools/list.
pub fn core_tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "search",
            "description": "Hybrid semantic+keyword search over the index. Returns structured hits {file,line,content}. Exact symbol-name tokens are boosted; results diversify by file (max_per_file). Content is capped for tokens.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)", "default": 10 },
                    "path": { "type": "string", "description": "Subdirectory to search within (e.g., 'src/api')" },
                    "glob": { "type": "string", "description": "File pattern on path or basename (e.g., '*.rs', 'src/**/*.ts')" },
                    "include_scores": { "type": "boolean", "description": "Include score/rank_score/score_source fields", "default": false },
                    "preview_mode": { "type": "boolean", "description": "Short preview (2-3 lines) instead of full chunk", "default": false },
                    "min_score": { "type": "number", "description": "Minimum score (vector cosine when available)", "default": 0.0 },
                    "include_context": { "type": "boolean", "description": "Include matched symbol/context field", "default": true },
                    "max_per_file": { "type": "integer", "description": "Max hits kept per file after ranking (default 3)", "default": 3 }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "search_symbols",
            "description": "Search symbols by name pattern. Returns structured list [{file,line,kind,name,signature}].",
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
            "description": "Find references to a symbol. Prefers resolved target_symbol_id edges (fewer false positives on common names). Optional file disambiguates which definition when the name is shared.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to find references for" },
                    "file": { "type": "string", "description": "Defining file path to disambiguate (optional)" }
                },
                "required": ["symbol"]
            }
        }),
        json!({
            "name": "get_callers",
            "description": "Find callers of a symbol (incoming call/usage). Prefers resolved symbol ids; optional file disambiguates the definition.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to find callers for" },
                    "file": { "type": "string", "description": "Defining file path to disambiguate (optional)" }
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
            "name": "batch_operations",
            "description": "Execute multiple index search/read operations in one request (search, symbols, skeleton, references, function_context, types_only). Not for host FS ops.",
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
                                    "enum": ["search", "get_symbols", "skeleton", "get_references", "read_function_context", "read_types_only"],
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
            "name": "get_index_status",
            "description": "Get current index status with completeness metrics, file counts, and last sync information. Returns token-optimized status summaries.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "validate_index",
            "description": "Validate index integrity: detect files missing symbols, orphaned data, failed indexing, broken references, and embedding gaps. Returns issues with severity and remediation recommendations.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "reindex",
            "description": "Rebuild index. path= one file + resolve. force=true clears SQLite+Lance+chunks_fts, full_sync, resolve. cancel=true aborts in-flight force reindex. Progress: get_index_status.reindex.stage.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "force": { "type": "boolean", "description": "Full clear + full_sync + resolve (default false)", "default": false },
                    "path": { "type": "string", "description": "Single file path relative to project root" },
                    "cancel": { "type": "boolean", "description": "Cancel in-flight force reindex for this project", "default": false }
                }
            }
        })
    ]
}
