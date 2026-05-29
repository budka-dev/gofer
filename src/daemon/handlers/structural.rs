//! Structural search via tree-sitter queries.
//!
//! Lets the agent search for AST-shaped patterns instead of regex over text.
//! Examples: "all `.unwrap()` outside tests", "all `console.log` calls",
//! "all `panic!()` invocations". Catalog of presets covers common smells per
//! language; custom S-expression queries are supported via the `query` arg.

use super::common::{make_relative_pathbuf, resolve_path_buf, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;
use streaming_iterator::StreamingIterator;
use walkdir::WalkDir;

/// Preset = (id, language, S-expression query, description).
///
/// Queries use tree-sitter syntax. `@m`-prefix captures get returned to the
/// caller as `match_text`/`match_kind`. Predicates like `#eq?` and `#match?`
/// constrain captures to specific values.
const PRESETS: &[(&str, &str, &str, &str)] = &[
    // --- Rust ---
    (
        "rust_unwrap",
        "rust",
        r#"(call_expression
            function: (field_expression
                field: (field_identifier) @m (#eq? @m "unwrap"))) @hit"#,
        "Calls to .unwrap() — panic on Err/None in production code path",
    ),
    (
        "rust_expect",
        "rust",
        r#"(call_expression
            function: (field_expression
                field: (field_identifier) @m (#eq? @m "expect"))) @hit"#,
        "Calls to .expect() — panic with message on Err/None",
    ),
    (
        "rust_panic",
        "rust",
        r#"(macro_invocation
            macro: (identifier) @name (#eq? @name "panic")) @hit"#,
        "panic!() invocations",
    ),
    (
        "rust_todo_unimplemented",
        "rust",
        r#"(macro_invocation
            macro: (identifier) @name
            (#match? @name "^(todo|unimplemented)$")) @hit"#,
        "todo!() / unimplemented!() markers — pending work",
    ),
    (
        "rust_dbg",
        "rust",
        r#"(macro_invocation
            macro: (identifier) @name (#eq? @name "dbg")) @hit"#,
        "dbg!() calls — debug prints that should be removed",
    ),
    (
        "rust_println",
        "rust",
        r#"(macro_invocation
            macro: (identifier) @name
            (#match? @name "^(println|eprintln|print|eprint)$")) @hit"#,
        "println!/eprintln! calls — likely debug output in lib code",
    ),
    (
        "rust_clone",
        "rust",
        r#"(call_expression
            function: (field_expression
                field: (field_identifier) @m (#eq? @m "clone"))) @hit"#,
        "Explicit .clone() calls — potential performance smell",
    ),
    // --- TypeScript / JavaScript ---
    (
        "ts_any",
        "typescript",
        r#"((predefined_type) @hit (#eq? @hit "any"))"#,
        "Usage of `any` type — TypeScript escape hatch",
    ),
    (
        "ts_console_log",
        "typescript",
        r#"(call_expression
            function: (member_expression
                object: (identifier) @obj
                property: (property_identifier) @prop
                (#eq? @obj "console")
                (#match? @prop "^(log|debug|info)$"))) @hit"#,
        "console.log/debug/info — likely debug output",
    ),
    (
        "ts_ts_ignore",
        "typescript",
        r#"((comment) @c (#match? @c "@ts-ignore|@ts-expect-error|@ts-nocheck")) @hit"#,
        "TypeScript suppression comments — skipped type checks",
    ),
    (
        "ts_debugger",
        "typescript",
        r#"(debugger_statement) @hit"#,
        "`debugger` statements — must not ship to production",
    ),
    (
        "ts_non_null",
        "typescript",
        r#"(non_null_expression) @hit"#,
        "Non-null assertions (`x!`) — bypass null checks, potential runtime crash",
    ),
    // --- Python ---
    (
        "py_print",
        "python",
        r#"(call function: (identifier) @name (#eq? @name "print")) @hit"#,
        "print() calls — likely debug output in lib code",
    ),
    (
        "py_bare_except",
        "python",
        r#"(except_clause
            !value) @hit"#,
        "Bare `except:` clauses — swallows all exceptions including KeyboardInterrupt",
    ),
    (
        "py_breakpoint",
        "python",
        r#"(call function: (identifier) @name (#eq? @name "breakpoint")) @hit"#,
        "breakpoint() calls — leftover debugger",
    ),
    // --- Go ---
    // NOTE: Go grammar may not be installed in every environment. These compile
    // and run only when `gofer install-lang go` has been done. Untested in CI
    // when the grammar is absent — verified by the compile-test only if loaded.
    (
        "go_panic",
        "go",
        r#"(call_expression
            function: (identifier) @name (#eq? @name "panic")) @hit"#,
        "panic() calls — abrupt termination in production code",
    ),
    (
        "go_fmt_print",
        "go",
        r#"(call_expression
            function: (selector_expression
                operand: (identifier) @pkg
                field: (field_identifier) @fn)
            (#eq? @pkg "fmt")
            (#match? @fn "^(Print|Println|Printf)$")) @hit"#,
        "fmt.Print* calls — likely debug output in lib code",
    ),
];

/// Maximum bytes per file we'll attempt to parse. Skips large generated files.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub async fn tool_structural_search(args: Value, ctx: &ToolContext) -> Result<Value> {
    let preset = args.get("preset").and_then(|v| v.as_str());
    let custom_query = args.get("query").and_then(|v| v.as_str());
    let lang_filter = args.get("language").and_then(|v| v.as_str());
    let path_filter = args.get("path").and_then(|v| v.as_str());
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(2000) as usize;
    let include_text = args
        .get("include_text")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if preset.is_none() && custom_query.is_none() {
        return Ok(json!({
            "presets": PRESETS.iter()
                .map(|(id, lang, _, desc)| json!({
                    "id": id,
                    "language": lang,
                    "description": desc,
                }))
                .collect::<Vec<_>>(),
            "usage": "Pass `preset` (id from above) OR `query` (custom tree-sitter S-expression). `language` required when `query` is custom.",
        }));
    }

    // Resolve which (language, query_string) pairs we'll run.
    let mut targets: Vec<(String, String, Option<&'static str>)> = Vec::new();

    if let Some(p) = preset {
        let entry = PRESETS
            .iter()
            .find(|(id, _, _, _)| *id == p)
            .ok_or_else(|| {
                GoferError::InvalidParams(format!(
                    "Unknown preset '{}'. Call without args to see catalog.",
                    p
                ))
            })?;
        targets.push((entry.1.to_string(), entry.2.to_string(), Some(entry.0)));
    } else if let Some(q) = custom_query {
        let lang = lang_filter.ok_or_else(|| {
            GoferError::InvalidParams(
                "`language` is required when using a custom `query`".into(),
            )
        })?;
        targets.push((lang.to_string(), q.to_string(), None));
    }

    // Optional language filter — drop presets that don't match.
    if let Some(lang) = lang_filter {
        targets.retain(|(l, _, _)| l == lang);
        if targets.is_empty() {
            return Err(GoferError::InvalidParams(format!(
                "No target queries for language '{}'",
                lang
            ))
            .into());
        }
    }

    let search_root = if let Some(p) = path_filter {
        resolve_path_buf(&ctx.root_path, p)?
    } else {
        ctx.root_path.as_ref().clone()
    };

    if !search_root.exists() {
        return Err(GoferError::InvalidParams(format!(
            "Directory not found: {:?}",
            search_root
        ))
        .into());
    }

    // Pre-compile queries per language. Compilation errors surface here, not
    // per-file, so a bad custom query fails fast with a useful message.
    let mut compiled: Vec<(String, Arc<tree_sitter::Query>, Option<&'static str>)> = Vec::new();
    for (lang_name, query_str, preset_id) in &targets {
        let loaded = crate::indexer::parser::LANG_MANAGER
            .get_language(lang_name)
            .ok_or_else(|| {
                GoferError::InvalidParams(format!(
                    "Language '{}' not loaded. Use `gofer install-lang {}`.",
                    lang_name, lang_name
                ))
            })?;
        let query = tree_sitter::Query::new(&loaded.language, query_str).map_err(|e| {
            GoferError::InvalidParams(format!(
                "Query compile error for {}: {} (line {}, column {})",
                lang_name, e.message, e.row, e.column
            ))
        })?;
        compiled.push((lang_name.clone(), Arc::new(query), *preset_id));
    }

    let mut hits: Vec<Value> = Vec::with_capacity(max_results.min(128));
    let mut files_scanned = 0usize;
    let mut files_matched = 0usize;
    let mut truncated = false;

    'outer: for entry in WalkDir::new(&search_root)
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
        let path = entry.path();

        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None => continue,
        };

        let lang_name = match crate::indexer::parser::LANG_MANAGER.get_language_by_ext(ext) {
            Some(l) => l,
            None => continue,
        };

        // Only run queries that target this file's language.
        let queries_for_file: Vec<_> = compiled
            .iter()
            .filter(|(l, _, _)| l == &lang_name)
            .collect();
        if queries_for_file.is_empty() {
            continue;
        }

        if entry
            .metadata()
            .map(|m| m.len() > MAX_FILE_BYTES)
            .unwrap_or(false)
        {
            continue;
        }

        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        files_scanned += 1;

        let loaded = match crate::indexer::parser::LANG_MANAGER.get_language(&lang_name) {
            Some(l) => l,
            None => continue,
        };

        let tree = crate::indexer::parser::with_parser(|parser| {
            if parser.set_language(&loaded.language).is_err() {
                return None;
            }
            parser.parse(&content, None)
        });
        let tree = match tree {
            Some(t) => t,
            None => continue,
        };

        let mut file_had_hit = false;

        for (_, query, preset_id) in &queries_for_file {
            let mut cursor = tree_sitter::QueryCursor::new();
            let capture_names = query.capture_names();
            let mut matches = cursor.matches(query, tree.root_node(), content.as_bytes());

            while let Some(m) = matches.next() {
                let hit_capture = m
                    .captures
                    .iter()
                    .find(|c| capture_names[c.index as usize] == "hit")
                    .or_else(|| m.captures.first());
                let Some(cap) = hit_capture else { continue };

                let node = cap.node;
                let start_pos = node.start_position();
                let end_pos = node.end_position();
                let rel_path = make_relative_pathbuf(&ctx.root_path, path);

                let text_snippet = if include_text {
                    let raw = &content[node.byte_range()];
                    // Collapse multi-line snippets to keep responses tight.
                    let collapsed = raw
                        .split('\n')
                        .map(|s| s.trim_end())
                        .collect::<Vec<_>>()
                        .join(" \\n ");
                    if collapsed.len() > 200 {
                        format!("{}…", &collapsed[..200])
                    } else {
                        collapsed
                    }
                } else {
                    String::new()
                };

                let mut hit = json!({
                    "file": rel_path,
                    "line_start": start_pos.row + 1,
                    "line_end": end_pos.row + 1,
                    "col_start": start_pos.column + 1,
                    "col_end": end_pos.column + 1,
                    "language": lang_name,
                });
                if include_text {
                    hit["text"] = json!(text_snippet);
                }
                if let Some(p) = preset_id {
                    hit["preset"] = json!(p);
                }

                hits.push(hit);
                file_had_hit = true;
                if hits.len() >= max_results {
                    truncated = true;
                    break 'outer;
                }
            }
        }

        if file_had_hit {
            files_matched += 1;
        }
    }

    Ok(json!({
        "hits": hits,
        "total_hits": hits.len(),
        "files_scanned": files_scanned,
        "files_matched": files_matched,
        "truncated": truncated,
        "max_results": max_results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every preset's query must compile against its grammar — when the grammar
    /// is installed. Skips (does not fail) presets whose grammar is absent, so
    /// CI without Go still passes while Rust/Python/TS are fully verified.
    #[test]
    fn all_presets_compile() {
        let mut checked = 0;
        let mut skipped = Vec::new();
        for (id, lang, query, _desc) in PRESETS {
            match crate::indexer::parser::LANG_MANAGER.get_language(lang) {
                Some(loaded) => {
                    let res = tree_sitter::Query::new(&loaded.language, query);
                    assert!(
                        res.is_ok(),
                        "preset '{}' ({}) failed to compile: {:?}",
                        id,
                        lang,
                        res.err()
                    );
                    checked += 1;
                }
                None => skipped.push((*id, *lang)),
            }
        }
        // At least the always-installed grammars must have been exercised.
        assert!(checked > 0, "no grammars available to verify presets");
        if !skipped.is_empty() {
            eprintln!("preset compile-test skipped (grammar absent): {skipped:?}");
        }
    }

    /// Each preset id must be unique — agents reference them by id.
    #[test]
    fn preset_ids_unique() {
        let mut seen = std::collections::HashSet::new();
        for (id, _, _, _) in PRESETS {
            assert!(seen.insert(*id), "duplicate preset id: {id}");
        }
    }

    /// Run a preset's query against a snippet and return number of @hit matches.
    /// Returns None if the grammar isn't installed (so the test self-skips).
    fn run_preset(preset_id: &str, src: &str) -> Option<usize> {
        let (_, lang, query_str, _) = PRESETS.iter().find(|(id, _, _, _)| *id == preset_id)?;
        let loaded = crate::indexer::parser::LANG_MANAGER.get_language(lang)?;
        let query = tree_sitter::Query::new(&loaded.language, query_str).expect("compile");
        let tree = crate::indexer::parser::with_parser(|p| {
            p.set_language(&loaded.language).unwrap();
            p.parse(src, None)
        })
        .unwrap();
        let hit_idx = query.capture_names().iter().position(|n| *n == "hit");
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), src.as_bytes());
        let mut count = 0;
        while let Some(m) = matches.next() {
            let has_hit = match hit_idx {
                Some(hi) => m.captures.iter().any(|c| c.index as usize == hi),
                None => !m.captures.is_empty(),
            };
            if has_hit {
                count += 1;
            }
        }
        Some(count)
    }

    #[test]
    fn ts_any_matches() {
        // This preset shipped broken (bad S-expression) until the compile-test
        // caught it. Verify it now actually matches `any`.
        let src = "function f(x: any): any { let y: number = 1; return x; }";
        if let Some(n) = run_preset("ts_any", src) {
            assert_eq!(n, 2, "expected 2 `any` hits");
        }
    }

    #[test]
    fn rust_unwrap_matches() {
        let src = "fn f(o: Option<i32>) -> i32 { let a = o.unwrap(); let b = o.unwrap(); a + b }";
        if let Some(n) = run_preset("rust_unwrap", src) {
            assert_eq!(n, 2);
        }
    }

    #[test]
    fn py_print_matches() {
        let src = "def f():\n    print('a')\n    print('b')\n    x = 1\n";
        if let Some(n) = run_preset("py_print", src) {
            assert_eq!(n, 2);
        }
    }

    #[test]
    fn ts_console_log_matches() {
        let src = "function f() { console.log('a'); console.error('b'); console.debug('c'); }";
        if let Some(n) = run_preset("ts_console_log", src) {
            // log + debug match; error is not in the (log|debug|info) set.
            assert_eq!(n, 2);
        }
    }
}
