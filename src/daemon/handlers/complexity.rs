//! Cyclomatic complexity + size metrics via tree-sitter.
//!
//! Walks function-like nodes and counts decision points (McCabe-style:
//! complexity = 1 + decision_points). Also reports line count, parameter
//! count, and max nesting depth so the agent knows *where* to look first when
//! debugging or refactoring.
//!
//! Decision points counted per function subtree:
//! - branch constructs (if/elif, match/switch arms, loops, except/catch)
//! - short-circuit operators (`&&`, `||`, `??`)
//! - ternary / conditional expressions
//!
//! Nested named functions are NOT descended into (they get their own entry);
//! closures/lambdas ARE descended into and attributed to the enclosing fn.

use super::common::{make_relative_pathbuf, resolve_path_buf, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};
use tree_sitter::Node;
use walkdir::WalkDir;

/// Function-like node kinds across supported grammars. These both seed the
/// per-function entries and act as recursion boundaries for decision counting.
const FUNCTION_KINDS: &[&str] = &[
    // Rust
    "function_item",
    // TS / JS
    "function_declaration",
    "method_definition",
    "generator_function_declaration",
    // Python
    "function_definition",
    // Go
    "method_declaration",
];

/// Named node kinds that represent a decision point. Union across grammars —
/// kinds are grammar-unique enough that there's no cross-language bleed.
const DECISION_KINDS: &[&str] = &[
    // Rust
    "if_expression",
    "match_arm",
    "while_expression",
    "for_expression",
    "loop_expression",
    // Python
    "if_statement",
    "elif_clause",
    "while_statement",
    "for_statement",
    "except_clause",
    "conditional_expression",
    "boolean_operator",
    "if_clause", // comprehension guard
    // TS / JS
    "for_in_statement",
    "do_statement",
    "switch_case", // `default` is `switch_default`, not counted
    "catch_clause",
    "ternary_expression",
    // Go
    "expression_case",
    "type_case",
    "communication_case",
    "for_clause",
];

/// Anonymous operator tokens that introduce a branch (short-circuit eval).
const DECISION_OPERATORS: &[&str] = &["&&", "||", "??"];

/// Closure/lambda kinds — descended into (counted toward enclosing function),
/// but never reported as standalone entries.
const CLOSURE_KINDS: &[&str] = &["closure_expression", "arrow_function", "lambda"];

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub async fn tool_complexity(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_arg = args.get("file").and_then(|v| v.as_str());
    let path_arg = args.get("path").and_then(|v| v.as_str());
    let min_complexity = args
        .get("min_complexity")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let sort = args.get("sort").and_then(|v| v.as_str()).unwrap_or("complexity");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(1000) as usize;

    let mut entries: Vec<FnMetrics> = Vec::new();
    let mut files_scanned = 0usize;

    if let Some(f) = file_arg {
        let abs = resolve_path_buf(&ctx.root_path, f)?;
        if let Some(mut found) = analyze_file(ctx, &abs).await? {
            entries.append(&mut found);
            files_scanned = 1;
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
        for entry in WalkDir::new(&root)
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
            if let Some(mut found) = analyze_file(ctx, entry.path()).await? {
                entries.append(&mut found);
                files_scanned += 1;
            }
        }
    }

    // Aggregate stats before filtering/truncation.
    let total_functions = entries.len();
    let avg_complexity = if total_functions > 0 {
        entries.iter().map(|e| e.complexity as f64).sum::<f64>() / total_functions as f64
    } else {
        0.0
    };
    let max_complexity = entries.iter().map(|e| e.complexity).max().unwrap_or(0);

    // Filter by threshold.
    entries.retain(|e| e.complexity >= min_complexity);

    // Sort.
    match sort {
        "lines" => entries.sort_by(|a, b| b.lines.cmp(&a.lines)),
        "nesting" => entries.sort_by(|a, b| b.max_nesting.cmp(&a.max_nesting)),
        "name" => entries.sort_by(|a, b| a.name.cmp(&b.name)),
        _ => entries.sort_by(|a, b| b.complexity.cmp(&a.complexity)),
    }

    let matched = entries.len();
    let truncated = matched > limit;
    entries.truncate(limit);

    let functions: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "file": e.file,
                "line_start": e.line_start,
                "line_end": e.line_end,
                "complexity": e.complexity,
                "rating": rating(e.complexity),
                "lines": e.lines,
                "params": e.params,
                "max_nesting": e.max_nesting,
            })
        })
        .collect();

    Ok(json!({
        "functions": functions,
        "files_scanned": files_scanned,
        "total_functions": total_functions,
        "matched": matched,
        "truncated": truncated,
        "stats": {
            "avg_complexity": (avg_complexity * 100.0).round() / 100.0,
            "max_complexity": max_complexity,
        },
        "filters": {
            "min_complexity": min_complexity,
            "sort": sort,
            "limit": limit,
        },
        "scale": "1-5 simple, 6-10 moderate, 11-20 complex, 21+ very complex (refactor candidate)",
    }))
}

struct FnMetrics {
    name: String,
    file: String,
    line_start: usize,
    line_end: usize,
    complexity: u32,
    lines: usize,
    params: usize,
    max_nesting: u32,
}

/// Returns None when the file's language isn't loaded or doesn't parse.
async fn analyze_file(
    ctx: &ToolContext,
    path: &std::path::Path,
) -> Result<Option<Vec<FnMetrics>>> {
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
    let mut out = Vec::new();
    collect_functions(tree.root_node(), &content, &rel, &mut out);
    Ok(Some(out))
}

/// Walk the tree finding function-like nodes. Each becomes an entry; its
/// subtree is scanned for decision points (stopping at nested functions).
fn collect_functions(node: Node, src: &str, file: &str, out: &mut Vec<FnMetrics>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if FUNCTION_KINDS.contains(&child.kind()) {
            let name = function_name(child, src);
            let line_start = child.start_position().row + 1;
            let line_end = child.end_position().row + 1;
            let mut decisions = 0u32;
            let mut max_nesting = 0u32;
            count_decisions(child, &mut decisions, 0, &mut max_nesting, true);
            out.push(FnMetrics {
                name,
                file: file.to_string(),
                line_start,
                line_end,
                complexity: 1 + decisions,
                lines: line_end.saturating_sub(line_start) + 1,
                params: count_params(child),
                max_nesting,
            });
            // Still descend to find nested named functions (separate entries).
            collect_functions(child, src, file, out);
        } else {
            collect_functions(child, src, file, out);
        }
    }
}

/// Recursively count decision points in a function subtree.
///
/// `is_root` is true for the function node itself (so we don't bail on it as a
/// nested-function boundary). For any *other* function-like node we stop —
/// that nested function is reported separately. Closures are descended into.
fn count_decisions(
    node: Node,
    decisions: &mut u32,
    depth: u32,
    max_nesting: &mut u32,
    is_root: bool,
) {
    if !is_root && FUNCTION_KINDS.contains(&node.kind()) {
        return; // nested named function — its own entry
    }

    let kind = node.kind();
    let mut next_depth = depth;

    if DECISION_KINDS.contains(&kind) {
        *decisions += 1;
        next_depth = depth + 1;
        if next_depth > *max_nesting {
            *max_nesting = next_depth;
        }
    } else if DECISION_OPERATORS.contains(&kind) {
        *decisions += 1;
    }

    // child(i) includes anonymous nodes (operators), unlike named_children.
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i as u32) {
            count_decisions(child, decisions, next_depth, max_nesting, false);
        }
    }
}

/// Extract a readable name for a function-like node, falling back to
/// "<anonymous>" / "<closure>".
fn function_name(node: Node, src: &str) -> String {
    if let Some(name_node) = node.child_by_field_name("name") {
        return src[name_node.byte_range()].to_string();
    }
    if CLOSURE_KINDS.contains(&node.kind()) {
        return "<closure>".to_string();
    }
    "<anonymous>".to_string()
}

/// Count formal parameters by locating the parameters node and counting its
/// named children.
fn count_params(node: Node) -> usize {
    let params_node = node.child_by_field_name("parameters").or_else(|| {
        // Some grammars name it differently; scan direct children.
        let mut cursor = node.walk();
        let mut found = None;
        for c in node.children(&mut cursor) {
            let k = c.kind();
            if k == "parameters" || k == "parameter_list" || k == "formal_parameters" {
                found = Some(c);
                break;
            }
        }
        found
    });
    match params_node {
        Some(p) => {
            let mut cursor = p.walk();
            p.named_children(&mut cursor)
                .filter(|c| {
                    // Skip `self`/`this` receiver and type-only nodes where they
                    // appear as siblings (Go method receiver is separate).
                    !matches!(c.kind(), "self_parameter")
                })
                .count()
        }
        None => 0,
    }
}

fn rating(complexity: u32) -> &'static str {
    match complexity {
        0..=5 => "simple",
        6..=10 => "moderate",
        11..=20 => "complex",
        _ => "very_complex",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `src` as `lang` and return the per-function metrics.
    fn analyze(src: &str, lang: &str) -> Vec<FnMetrics> {
        let loaded = crate::indexer::parser::LANG_MANAGER
            .get_language(lang)
            .expect("language grammar must be installed for this test");
        let tree = crate::indexer::parser::with_parser(|parser| {
            parser.set_language(&loaded.language).unwrap();
            parser.parse(src, None)
        })
        .expect("parse");
        let mut out = Vec::new();
        collect_functions(tree.root_node(), src, "test.rs", &mut out);
        out
    }

    fn complexity_of(metrics: &[FnMetrics], name: &str) -> u32 {
        metrics
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("function '{}' not found", name))
            .complexity
    }

    #[test]
    fn rust_straight_line_is_one() {
        let src = "fn f() { let x = 1; let y = 2; println!(\"{}\", x + y); }";
        let m = analyze(src, "rust");
        assert_eq!(complexity_of(&m, "f"), 1);
    }

    #[test]
    fn rust_single_if_is_two() {
        let src = "fn f(x: i32) -> i32 { if x > 0 { 1 } else { 2 } }";
        let m = analyze(src, "rust");
        // 1 base + 1 if. The `else` is not a decision point.
        assert_eq!(complexity_of(&m, "f"), 2);
    }

    #[test]
    fn rust_else_if_chain() {
        let src = "fn f(x: i32) -> i32 { if x > 2 { 1 } else if x > 1 { 2 } else { 3 } }";
        let m = analyze(src, "rust");
        // 1 base + 2 if_expression (the else-if is a nested if_expression).
        assert_eq!(complexity_of(&m, "f"), 3);
    }

    #[test]
    fn rust_short_circuit_operators() {
        let src = "fn f(a: bool, b: bool, c: bool) -> bool { if a && b || c { true } else { false } }";
        let m = analyze(src, "rust");
        // 1 base + 1 if + 1 `&&` + 1 `||` = 4.
        assert_eq!(complexity_of(&m, "f"), 4);
    }

    #[test]
    fn rust_match_arms() {
        let src = "fn f(x: i32) -> i32 { match x { 0 => 1, 1 => 2, _ => 3 } }";
        let m = analyze(src, "rust");
        // 1 base + 3 match_arm.
        assert_eq!(complexity_of(&m, "f"), 4);
    }

    #[test]
    fn rust_loop_and_while() {
        let src = "fn f() { for _ in 0..10 { } while true { break; } loop { break; } }";
        let m = analyze(src, "rust");
        // 1 base + for + while + loop = 4.
        assert_eq!(complexity_of(&m, "f"), 4);
    }

    #[test]
    fn rust_nested_named_fn_is_separate_entry() {
        let src = "fn outer(x: i32) -> i32 { fn inner(y: i32) -> i32 { if y > 0 { 1 } else { 2 } } inner(x) }";
        let m = analyze(src, "rust");
        // outer has no decisions of its own (the if lives in inner).
        assert_eq!(complexity_of(&m, "outer"), 1);
        assert_eq!(complexity_of(&m, "inner"), 2);
    }

    #[test]
    fn rust_closure_counts_toward_enclosing() {
        let src = "fn f(v: Vec<i32>) -> usize { v.iter().filter(|x| if **x > 0 { true } else { false }).count() }";
        let m = analyze(src, "rust");
        // The closure's `if` is attributed to `f` (closures aren't separate
        // entries). 1 base + 1 if = 2.
        assert_eq!(complexity_of(&m, "f"), 2);
    }

    #[test]
    fn rust_param_count() {
        let src = "fn f(a: i32, b: i32, c: i32) {}";
        let m = analyze(src, "rust");
        let f = m.iter().find(|x| x.name == "f").unwrap();
        assert_eq!(f.params, 3);
    }

    // === Cross-language verification ===

    #[test]
    fn python_if_elif_and_bool_ops() {
        let src = "def f(x):\n    if x > 2 and x < 10:\n        return 1\n    elif x == 0:\n        return 2\n    return 3\n";
        let m = analyze(src, "python");
        // 1 base + if + elif + `and` (boolean_operator) = 4.
        assert_eq!(complexity_of(&m, "f"), 4);
    }

    #[test]
    fn python_for_and_while() {
        let src = "def f(items):\n    for i in items:\n        while i > 0:\n            i -= 1\n";
        let m = analyze(src, "python");
        assert_eq!(complexity_of(&m, "f"), 3); // 1 + for + while
    }

    #[test]
    fn python_param_count() {
        let src = "def f(a, b, c):\n    pass\n";
        let m = analyze(src, "python");
        assert_eq!(m.iter().find(|x| x.name == "f").unwrap().params, 3);
    }

    #[test]
    fn ts_if_ternary_and_short_circuit() {
        let src = "function f(a: boolean, b: boolean): number { if (a && b) { return a ? 1 : 2; } return 3; }";
        let m = analyze(src, "typescript");
        // 1 base + if + `&&` + ternary = 4.
        assert_eq!(complexity_of(&m, "f"), 4);
    }

    #[test]
    fn ts_switch_cases() {
        let src = "function f(x: number): number { switch (x) { case 1: return 1; case 2: return 2; default: return 3; } }";
        let m = analyze(src, "typescript");
        // 1 base + 2 switch_case (default not counted) = 3.
        assert_eq!(complexity_of(&m, "f"), 3);
    }
}
