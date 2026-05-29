//! Detect statically unreachable code via tree-sitter.
//!
//! Strategy: within a single block, any statement that follows an
//! *unconditional terminator* (return / break / continue / throw / raise /
//! panic-like macro) in the same statement list is unreachable.
//!
//! Crucially, only **direct siblings** of a terminator count. A `return`
//! inside an `if` branch does NOT make code after the `if` unreachable —
//! that code runs when the condition is false. This keeps false positives
//! near zero.
//!
//! Out of scope (planned): unreachable match/switch arms after a catch-all,
//! conditions provably always-false, fns whose every branch diverges.

use super::common::{make_relative_pathbuf, resolve_path_buf, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};
use tree_sitter::Node;
use walkdir::WalkDir;

/// Block / statement-list node kinds across grammars.
const BLOCK_KINDS: &[&str] = &[
    "block",            // rust, python, go
    "statement_block",  // ts / js
];

/// Comment-ish kinds we skip when scanning for unreachable statements — a
/// comment after `return` isn't "dead code".
const SKIP_KINDS: &[&str] = &["line_comment", "block_comment", "comment"];

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub async fn tool_find_unreachable(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_arg = args.get("file").and_then(|v| v.as_str());
    let path_arg = args.get("path").and_then(|v| v.as_str());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(2000) as usize;

    let mut hits: Vec<Value> = Vec::new();
    let mut files_scanned = 0usize;
    let mut truncated = false;

    if let Some(f) = file_arg {
        let abs = resolve_path_buf(&ctx.root_path, f)?;
        if let Some(found) = analyze_file(ctx, &abs).await? {
            files_scanned = 1;
            for h in found {
                hits.push(h);
                if hits.len() >= limit {
                    truncated = true;
                    break;
                }
            }
        }
    } else {
        let root = if let Some(p) = path_arg {
            resolve_path_buf(&ctx.root_path, p)?
        } else {
            ctx.root_path.as_ref().clone()
        };
        if !root.exists() {
            return Err(
                GoferError::InvalidParams(format!("Path not found: {:?}", root)).into(),
            );
        }
        'outer: for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(|e| {
                let s = e.path().to_string_lossy();
                !s.contains("/.git/")
                    && !s.contains("/node_modules/")
                    && !s.contains("/target/")
                    && !s.contains("/dist/")
                    && !s.contains("/.gofer/")
            })
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if entry
                .metadata()
                .map(|m| m.len() > MAX_FILE_BYTES)
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(found) = analyze_file(ctx, entry.path()).await? {
                files_scanned += 1;
                for h in found {
                    hits.push(h);
                    if hits.len() >= limit {
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }
    }

    Ok(json!({
        "unreachable": hits,
        "total": hits.len(),
        "files_scanned": files_scanned,
        "truncated": truncated,
        "limit": limit,
    }))
}

async fn analyze_file(ctx: &ToolContext, path: &std::path::Path) -> Result<Option<Vec<Value>>> {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return Ok(None),
    };
    let lang_name = match crate::indexer::parser::LANG_MANAGER.get_language_by_ext(ext) {
        Some(l) => l,
        None => return Ok(None),
    };
    let loaded = match crate::indexer::parser::LANG_MANAGER.get_language(&lang_name) {
        Some(l) => l,
        None => return Ok(None),
    };
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let tree = crate::indexer::parser::with_parser(|parser| {
        if parser.set_language(&loaded.language).is_err() {
            return None;
        }
        parser.parse(&content, None)
    });
    let tree = match tree {
        Some(t) => t,
        None => return Ok(None),
    };

    let rel = make_relative_pathbuf(&ctx.root_path, path);
    let mut hits = Vec::new();
    scan_block(tree.root_node(), &content, &rel, &lang_name, &mut hits);
    Ok(Some(hits))
}

/// Recursively walk the tree. For every block node, scan its direct statement
/// children for an unconditional terminator and flag everything after it.
fn scan_block(node: Node, src: &str, file: &str, lang: &str, hits: &mut Vec<Value>) {
    if BLOCK_KINDS.contains(&node.kind()) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();

        let mut terminator: Option<(usize, &'static str)> = None;
        for (idx, child) in children.iter().enumerate() {
            if let Some(kind) = is_terminator(*child, src) {
                terminator = Some((idx, kind));
                break;
            }
        }

        if let Some((term_idx, term_kind)) = terminator {
            let term_line = children[term_idx].start_position().row + 1;
            for child in children.iter().skip(term_idx + 1) {
                if SKIP_KINDS.contains(&child.kind()) {
                    continue;
                }
                let snippet = &src[child.byte_range()];
                let first_line = snippet.lines().next().unwrap_or("").trim();
                let truncated_snippet = if first_line.len() > 120 {
                    format!("{}…", &first_line[..120])
                } else {
                    first_line.to_string()
                };
                hits.push(json!({
                    "file": file,
                    "line": child.start_position().row + 1,
                    "kind": child.kind(),
                    "code": truncated_snippet,
                    "reason": format!("after `{}` at line {}", term_kind, term_line),
                }));
            }
        }
    }

    // Recurse into children regardless — nested blocks scanned independently.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_block(child, src, file, lang, hits);
    }
}

/// If `node` is an unconditional control-flow terminator, return a short label
/// for it; otherwise None.
///
/// Handles statement-style terminators directly, unwraps `expression_statement`
/// wrappers, and recognises Rust panic-like macros.
fn is_terminator(node: Node, src: &str) -> Option<&'static str> {
    match node.kind() {
        "return_statement" => Some("return"),
        "raise_statement" => Some("raise"),
        "throw_statement" => Some("throw"),
        "break_statement" => Some("break"),
        "continue_statement" => Some("continue"),
        "goto_statement" => Some("goto"),
        // Rust expression-style control flow
        "return_expression" => Some("return"),
        "break_expression" => Some("break"),
        "continue_expression" => Some("continue"),
        // Rust wraps `return x;` etc. as expression_statement(return_expression)
        "expression_statement" => node
            .named_child(0)
            .and_then(|inner| is_terminator(inner, src)),
        // Rust panic-like macros: panic!/unreachable!/todo!/unimplemented!
        "macro_invocation" => {
            let macro_node = node.child_by_field_name("macro")?;
            let name = &src[macro_node.byte_range()];
            match name {
                "panic" => Some("panic!"),
                "unreachable" => Some("unreachable!"),
                "todo" => Some("todo!"),
                "unimplemented" => Some("unimplemented!"),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(src: &str, lang: &str) -> Vec<Value> {
        let loaded = crate::indexer::parser::LANG_MANAGER
            .get_language(lang)
            .expect("grammar installed");
        let tree = crate::indexer::parser::with_parser(|parser| {
            parser.set_language(&loaded.language).unwrap();
            parser.parse(src, None)
        })
        .expect("parse");
        let mut hits = Vec::new();
        scan_block(tree.root_node(), src, "t.rs", lang, &mut hits);
        hits
    }

    #[test]
    fn rust_code_after_return() {
        let src = "fn f() -> i32 { return 1; let x = 2; x }";
        let hits = find(src, "rust");
        // `let x = 2;` and the tail `x` are both unreachable.
        assert!(hits.len() >= 1, "expected unreachable after return, got {hits:?}");
        assert!(hits.iter().any(|h| h["code"].as_str().unwrap().contains("let x")));
    }

    #[test]
    fn rust_no_false_positive_for_return_in_if() {
        // return is inside the if-branch block; code after the if is reachable.
        let src = "fn f(c: bool) -> i32 { if c { return 1; } let x = 2; x }";
        let hits = find(src, "rust");
        assert!(hits.is_empty(), "false positive: {hits:?}");
    }

    #[test]
    fn rust_code_after_panic() {
        let src = "fn f() { panic!(\"boom\"); let x = 2; }";
        let hits = find(src, "rust");
        assert!(hits.iter().any(|h| h["code"].as_str().unwrap().contains("let x")));
        assert!(hits.iter().any(|h| h["reason"].as_str().unwrap().contains("panic!")));
    }

    #[test]
    fn rust_code_after_break_in_loop() {
        let src = "fn f() { loop { break; let dead = 1; } }";
        let hits = find(src, "rust");
        assert!(hits.iter().any(|h| h["code"].as_str().unwrap().contains("let dead")));
    }

    #[test]
    fn rust_clean_function_no_hits() {
        let src = "fn f(c: bool) -> i32 { let x = 1; if c { return x; } x + 1 }";
        let hits = find(src, "rust");
        assert!(hits.is_empty(), "unexpected: {hits:?}");
    }

    // === Cross-language verification ===

    #[test]
    fn python_code_after_return() {
        let src = "def f():\n    return 1\n    x = 2\n";
        let hits = find(src, "python");
        assert!(hits.iter().any(|h| h["code"].as_str().unwrap().contains("x = 2")));
    }

    #[test]
    fn python_code_after_raise() {
        let src = "def f():\n    raise ValueError()\n    cleanup()\n";
        let hits = find(src, "python");
        assert!(hits.iter().any(|h| h["reason"].as_str().unwrap().contains("raise")));
    }

    #[test]
    fn python_no_false_positive_return_in_if() {
        let src = "def f(c):\n    if c:\n        return 1\n    x = 2\n    return x\n";
        let hits = find(src, "python");
        assert!(hits.is_empty(), "false positive: {hits:?}");
    }

    #[test]
    fn ts_code_after_throw() {
        let src = "function f() { throw new Error('x'); const y = 2; }";
        let hits = find(src, "typescript");
        assert!(hits.iter().any(|h| h["reason"].as_str().unwrap().contains("throw")));
    }

    #[test]
    fn ts_no_false_positive_return_in_if() {
        let src = "function f(c: boolean): number { if (c) { return 1; } const x = 2; return x; }";
        let hits = find(src, "typescript");
        assert!(hits.is_empty(), "false positive: {hits:?}");
    }
}
