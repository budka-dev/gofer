use super::common::{make_relative, resolve_path, ToolContext};
use crate::error::GoferError;
use crate::storage::SqliteStorage;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashSet;
use streaming_iterator::StreamingIterator;
use walkdir::WalkDir;

pub async fn tool_read_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let start_line = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let end_line = args.get("end_line").and_then(|v| v.as_u64());

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }

    let file_path = &ctx.root_path.join(file);
    if !file_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", file)).into());
    }

    // Get file metadata for mtime check
    let file_metadata = tokio::fs::metadata(&file_path).await?;
    let current_mtime = file_metadata.modified().ok();

    // Try cache first, but validate mtime
    if let Some((cached_content, cached_mtime)) = ctx.cache.get_file_with_mtime(file).await {
        // Check if cached version is still valid
        let cache_valid = match (current_mtime, cached_mtime) {
            (Some(curr), Some(cached)) => curr == cached,
            _ => false,
        };

        if cache_valid {
            let lines: Vec<&str> = cached_content.lines().collect();
            let total_lines = lines.len();

            let end = end_line
                .map(|l| l as usize)
                .unwrap_or(total_lines)
                .min(total_lines);
            let start = start_line.max(1).min(end + 1) - 1; // 0-based index

            if start < end {
                let content = lines[start..end].join("\n");
                return Ok(json!({
                    "file": file,
                    "content": content,
                    "start_line": start + 1,
                    "end_line": end,
                    "total_lines": total_lines
                }));
            }
        }
    }

    // Read from disk if cache miss or invalid
    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| anyhow::anyhow!("File system error: {}", e))?;

    // Update cache
    ctx.cache.put_file(file.to_string(), content.clone()).await;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let end = end_line
        .map(|l| l as usize)
        .unwrap_or(total_lines)
        .min(total_lines);
    let start = start_line.max(1).min(end + 1) - 1; // 0-based index

    let result_content = if start < end {
        lines[start..end].join("\n")
    } else {
        String::new()
    };

    Ok(json!({
        "file": file,
        "content": result_content,
        "start_line": start + 1,
        "end_line": end,
        "total_lines": total_lines
    }))
}

pub async fn tool_file_exists(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str());

    if let Some(p) = file {
        let abs_path = resolve_path(&ctx.root_path, p);
        let exists = std::path::Path::new(&abs_path).exists();

        Ok(json!({
            "file": p,
            "exists": exists
        }))
    } else {
        Err(GoferError::InvalidParams("file is required".into()).into())
    }
}

pub async fn tool_skeleton(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }

    let include_private = args
        .get("include_private")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let include_tests = args
        .get("include_tests")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let file_path = &ctx.root_path.join(file);
    if !file_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", file)).into());
    }

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let language = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        _ => "unknown",
    };

    let original_content = tokio::fs::read_to_string(&file_path).await?;
    let original_lines = original_content.lines().count();
    let original_chars = original_content.len();

    let skeleton = crate::indexer::context::skeletonize_content(&original_content, ext);
    let mut skeleton = skeleton; // make mutable for filtering

    // Apply filters
    if !include_private {
        skeleton = filter_private_items(&skeleton, language);
    }

    if !include_tests {
        skeleton = filter_test_items(&skeleton, language);
    }

    let skeleton_lines = skeleton.lines().count();
    let skeleton_chars = skeleton.len();
    let reduction_percent = if original_chars > 0 {
        (original_chars - skeleton_chars) as f64 / original_chars as f64 * 100.0
    } else {
        0.0
    };

    // Count items in skeleton
    let items = count_skeleton_items(&skeleton, language);

    Ok(json!({
        "file_path": file,
        "language": language,
        "skeleton_content": skeleton,
        "stats": {
            "original_lines": original_lines,
            "original_chars": original_chars,
            "skeleton_lines": skeleton_lines,
            "skeleton_chars": skeleton_chars,
            "reduction_percent": format!("{:.1}", reduction_percent),
            "items_kept": items
        }
    }))
}

pub async fn tool_read_function_context(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let function = args.get("function").and_then(|v| v.as_str()).unwrap_or("");
    let include_types = args
        .get("include_types")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let include_imports = args
        .get("include_imports")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let include_callees = args
        .get("include_callees")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if file.is_empty() || function.is_empty() {
        return Err(GoferError::InvalidParams("File and function name are required".into()).into());
    }

    let file_path = &ctx.root_path.join(file);
    if !file_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", file)).into());
    }

    let content = tokio::fs::read_to_string(&file_path).await?;
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        _ => {
            return Err(GoferError::InvalidParams(format!("Unsupported language: {}", ext)).into())
        }
    };

    // Extract data from AST synchronously in a block to ensure !Send types (Node, etc) are dropped
    let (function_code, start_line, end_line, type_names, callee_names) = {
        let language = crate::indexer::parser::LANG_MANAGER.get_language(lang).expect("Lang not loaded").language.clone();

        let tree_opt = crate::indexer::parser::with_parser(|parser| {
            if parser.set_language(&language).is_err() {
                return None;
            }
            parser.parse(&content, None)
        });

        let tree = tree_opt.ok_or_else(|| anyhow::anyhow!("Failed to parse file"))?;

        let root = tree.root_node();
        let query_str = match lang {
            "rust" => format!(
                r#"[
                    (function_item name: (identifier) @name (#eq? @name "{name}"))
                    (function_signature_item name: (identifier) @name (#eq? @name "{name}"))
                ] @func"#,
                name = function
            ),
            // Match functions in any of the common TS/JS shapes the previous query missed:
            // class methods (instance/static), arrow functions and function expressions
            // assigned to const/let/var, and exported variants. Names live on either
            // `identifier` (top-level) or `property_identifier` (class methods).
            "typescript" | "javascript" => format!(
                r#"[
                    (function_declaration name: (identifier) @name (#eq? @name "{name}"))
                    (method_definition name: (property_identifier) @name (#eq? @name "{name}"))
                    (method_signature name: (property_identifier) @name (#eq? @name "{name}"))
                    (variable_declarator
                        name: (identifier) @name (#eq? @name "{name}")
                        value: [(arrow_function) (function_expression)])
                    (public_field_definition
                        name: (property_identifier) @name (#eq? @name "{name}")
                        value: [(arrow_function) (function_expression)])
                ] @func"#,
                name = function
            ),
            "python" => format!(
                r#"(function_definition name: (identifier) @name (#eq? @name "{name}")) @func"#,
                name = function
            ),
            "go" => format!(
                r#"[
                    (function_declaration name: (identifier) @name (#eq? @name "{name}"))
                    (method_declaration name: (field_identifier) @name (#eq? @name "{name}"))
                ] @func"#,
                name = function
            ),
            _ => return Err(anyhow::anyhow!("Unsupported language query")),
        };

        let query = tree_sitter::Query::new(&language, &query_str)
            .map_err(|e| anyhow::anyhow!("Query error: {}", e))?;

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, root, content.as_bytes());

        let mut func_node = None;
        while let Some(match_) = matches.next() {
            for c in match_.captures {
                if c.index == 1 {
                    // Assuming @func is 1
                    func_node = Some(c.node);
                    break;
                }
            }
            if func_node.is_some() {
                break;
            }
        }

        let function_node = if let Some(node) = func_node {
            node
        } else {
            return Err(GoferError::InvalidParams(format!(
                "Function '{}' not found in {}",
                function, file
            ))
            .into());
        };

        let start_byte = function_node.start_byte();
        let end_byte = function_node.end_byte();
        let function_code = content[start_byte..end_byte].to_string();
        let start_line = function_node.start_position().row + 1;
        let end_line = function_node.end_position().row + 1;

        let type_names = if include_types {
            collect_type_names(&function_node, &content, lang)?
        } else {
            HashSet::new()
        };

        let callee_names = if include_callees {
            collect_callee_names(&function_node, &content, lang)?
        } else {
            HashSet::new()
        };

        (
            function_code,
            start_line,
            end_line,
            type_names,
            callee_names,
        )
    };

    // Now call async functions
    let mut types = Vec::new();
    if include_types {
        types = resolve_types(type_names, &ctx.sqlite, file).await?;
    }

    let mut imports: Vec<String> = Vec::new();
    if include_imports {
        imports = vec![]; // Placeholder
    }

    let mut callees = Vec::new();
    if include_callees {
        callees = resolve_callees(callee_names, &content, lang).await?;
    }

    Ok(json!({
        "file": file,
        "function": function,
        "code": function_code,
        "start_line": start_line,
        "end_line": end_line,
        "referenced_types": types,
        "imports": imports,
        "callees": callees
    }))
}

pub async fn tool_read_types_only(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let kind_filter = args.get("kind").and_then(|v| v.as_str());
    let include_docs = args
        .get("include_docs")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }

    let file_path = &ctx.root_path.join(file);
    if !file_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", file)).into());
    }

    let content = tokio::fs::read_to_string(&file_path).await?;
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // For TS/JS, walk the AST so we catch TypeBox (`Type.Object(...)`) and Zod
    // (`z.object(...)`) schemas plus `Static<typeof X>` aliases — patterns that
    // the previous prefix-only line filter silently dropped on contract-style
    // files (slave-core-contracts/src/task.ts).
    if matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
        let blocks = extract_ts_type_blocks(&content, kind_filter, include_docs);
        return Ok(json!({
            "file": file,
            "kind": kind_filter,
            "total": blocks.len(),
            "types_content": blocks.join("\n\n"),
        }));
    }

    // Fallback for languages we don't AST-parse here (rust uses skeleton output)
    let skeleton = crate::indexer::context::skeletonize_content(&content, ext);
    let filtered_lines: Vec<&str> = skeleton
        .lines()
        .filter(|line| {
            let t = line.trim();
            let is_type = t.starts_with("struct ")
                || t.starts_with("enum ")
                || t.starts_with("interface ")
                || t.starts_with("type ")
                || t.starts_with("class ")
                || t.starts_with("pub struct ")
                || t.starts_with("pub enum ")
                || t.starts_with("pub type ");

            let is_doc = t.starts_with("///") || t.starts_with("/**");

            if let Some(k) = kind_filter {
                t.contains(k)
            } else {
                is_type || (include_docs && is_doc)
            }
        })
        .collect();

    Ok(json!({
        "file": file,
        "kind": kind_filter,
        "types_content": filtered_lines.join("\n")
    }))
}

/// Walk the TS/JS AST and collect every type-like declaration.
///
/// Recognises:
/// * `interface_declaration` → kind="interface"
/// * `type_alias_declaration` → kind="type" (with sub-classification "interface"
///   when the RHS is `Static<typeof X>` — TypeBox idiom for "interface from schema")
/// * `class_declaration` → kind="class"
/// * `enum_declaration` → kind="enum"
/// * `lexical_declaration` whose initializer is a TypeBox `Type.X(...)` or Zod
///   `z.X(...)` call → kind="schema" (treat as struct-equivalent so it shows up
///   under `kind: struct`/`kind: interface` filters too)
fn extract_ts_type_blocks(
    content: &str,
    kind_filter: Option<&str>,
    include_docs: bool,
) -> Vec<String> {
    let lang = match crate::indexer::parser::LANG_MANAGER.get_language("typescript") {
        Some(l) => l,
        None => return Vec::new(),
    };
    let tree = match crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&lang.language).ok()?;
        parser.parse(content, None)
    }) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let bytes = content.as_bytes();
    let root = tree.root_node();

    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        // export_statement wraps a real declaration; descend through it
        let actual = if child.kind() == "export_statement" {
            child
                .named_children(&mut child.walk())
                .next()
                .unwrap_or(child)
        } else {
            child
        };

        let kind_label = classify_type_node(&actual, content);
        let Some(kind_label) = kind_label else {
            continue;
        };

        if let Some(filter) = kind_filter {
            if !kind_matches(filter, kind_label) {
                continue;
            }
        }

        let start_byte = if include_docs {
            // Pull preceding doc comment lines into the block
            extend_back_to_docs(content, child.start_byte())
        } else {
            child.start_byte()
        };
        let end_byte = child.end_byte().min(bytes.len());
        if start_byte >= end_byte {
            continue;
        }
        let block = &content[start_byte..end_byte];
        out.push(block.trim_end().to_string());
    }

    out
}

fn classify_type_node<'a>(node: &tree_sitter::Node<'a>, src: &'a str) -> Option<&'static str> {
    match node.kind() {
        "interface_declaration" => Some("interface"),
        "type_alias_declaration" => {
            // Detect `Static<typeof XSchema>` — TypeBox's interface-equivalent.
            let val_text = node
                .child_by_field_name("value")
                .map(|n| &src[n.byte_range()])
                .unwrap_or("");
            if val_text.starts_with("Static<") || val_text.starts_with("Static <") {
                Some("interface")
            } else {
                Some("type")
            }
        }
        "class_declaration" | "abstract_class_declaration" => Some("class"),
        "enum_declaration" => Some("enum"),
        "lexical_declaration" | "variable_declaration" => {
            // Look for `const X = Type.Object({...})` / `z.object({...})`
            let text = &src[node.byte_range()];
            if is_typebox_or_zod_schema(text) {
                Some("schema")
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_typebox_or_zod_schema(text: &str) -> bool {
    // Cheap text check; precise enough for the call-shape patterns we care about
    text.contains("Type.Object(")
        || text.contains("Type.Union(")
        || text.contains("Type.Intersect(")
        || text.contains("Type.Array(")
        || text.contains("Type.Record(")
        || text.contains("z.object(")
        || text.contains("z.union(")
        || text.contains("z.array(")
        || text.contains("z.record(")
}

fn kind_matches(filter: &str, kind: &str) -> bool {
    let filter = filter.to_ascii_lowercase();
    if filter == kind {
        return true;
    }
    // Treat "struct" and "interface" as a family that includes schemas/aliases —
    // a TypeBox schema acts like a struct, a `Static<typeof X>` alias acts like
    // an interface — so callers asking for either get both.
    matches!(
        (filter.as_str(), kind),
        ("struct", "schema")
            | ("interface", "schema")
            | ("interface", "type")
            | ("type", "schema")
    )
}

fn extend_back_to_docs(content: &str, start: usize) -> usize {
    let bytes = content.as_bytes();
    let mut i = start;
    // Walk backwards over blank lines + // / /** ... */ comments
    while i > 0 {
        // Find the start of the current line by scanning back to a newline
        let line_start = content[..i].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line = &content[line_start..i];
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*")
            || trimmed.starts_with("*/")
        {
            i = line_start;
            if line_start == 0 {
                break;
            }
            // Step over the preceding newline so we keep going
            if bytes.get(line_start.saturating_sub(1)) == Some(&b'\n') {
                i = line_start.saturating_sub(1);
            }
        } else {
            break;
        }
    }
    // Skip over any leading newline we landed on
    while i < start && bytes.get(i) == Some(&b'\n') {
        i += 1;
    }
    i
}

pub async fn tool_context_bundle(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let skeleton = args
        .get("skeleton")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let skeleton_deps_only = args
        .get("skeleton_deps_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }

    let file_path = &ctx.root_path.join(file);
    if !file_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", file)).into());
    }

    let mut bundle = crate::indexer::context::create_bundle(file_path, depth).await;
    if skeleton {
        crate::indexer::context::skeletonize_bundle(&mut bundle);
    } else if skeleton_deps_only {
        crate::indexer::context::skeletonize_deps_only(&mut bundle);
    }

    let mode = if skeleton {
        "skeleton"
    } else if skeleton_deps_only {
        "skeleton_deps_only"
    } else {
        "full"
    };

    Ok(json!({
        "file": file,
        "mode": mode,
        "total_lines": bundle.total_lines,
        "total_tokens_estimate": bundle.total_tokens_estimate,
        "main_content": bundle.main_content,
        "dependencies": bundle.dependencies.iter().map(|dep| json!({
            "path": dep.path,
            "depth": dep.depth,
            "content": dep.content
        })).collect::<Vec<_>>()
    }))
}

pub async fn tool_find_files(args: Value, ctx: &ToolContext) -> Result<Value> {
    let pattern = args.get("pattern").and_then(|v| v.as_str());
    let path_filter = args.get("path").and_then(|v| v.as_str());
    // Surface truncation explicitly. Defaults match prior behaviour (100 files)
    // but the response now includes `total`/`truncated`/`limit`/`offset` so the
    // caller can paginate or widen the limit instead of guessing whether the
    // list was cut short.
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(10_000) as usize;
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let Some(pat) = pattern else {
        return Err(GoferError::InvalidParams("Pattern is required".into()).into());
    };

    let search_root = if let Some(p) = path_filter {
        ctx.root_path.join(p)
    } else {
        ctx.root_path.as_ref().clone()
    };

    if !search_root.exists() {
        return Ok(json!({
            "pattern": pat,
            "total": 0,
            "count": 0,
            "limit": limit,
            "offset": offset,
            "truncated": false,
            "files": []
        }));
    }

    let pat_string = pat.to_string();
    let ctx_root_path = ctx.root_path.clone();

    let all_files = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();

        let glob_pattern =
            glob::Pattern::new(&pat_string).map_err(|e| format!("Invalid glob pattern: {}", e))?;

        let walker = WalkDir::new(&search_root).into_iter();

        for entry in walker.filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }

            if entry.path().to_string_lossy().contains("/.git/") {
                continue;
            }

            let relative_to_search = entry
                .path()
                .strip_prefix(&search_root)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or("");

            if glob_pattern.matches(relative_to_search) {
                files.push(make_relative(
                    &ctx_root_path,
                    entry.path().to_str().unwrap_or(""),
                ));
            }
        }
        Ok::<_, String>(files)
    })
    .await
    .map_err(|e| GoferError::Internal(anyhow::anyhow!("Task panic: {}", e)))?
    .map_err(|e| GoferError::Internal(anyhow::anyhow!(e)))?;

    let total = all_files.len();
    let page: Vec<String> = all_files.into_iter().skip(offset).take(limit).collect();
    let count = page.len();
    let truncated = offset + count < total;

    Ok(json!({
        "pattern": pat,
        "total": total,
        "count": count,
        "limit": limit,
        "offset": offset,
        "truncated": truncated,
        "files": page
    }))
}

/// `grep` is now a thin alias around `tool_search_files`. Both used to walk the
/// tree with regex and produce the same shape of results — keeping two parallel
/// implementations meant fixing every bug twice. The wrapper translates the old
/// argument names (`pattern`/`path` → `regex_pattern`/`directory`) so existing
/// callers keep working.
pub async fn tool_grep(args: Value, ctx: &ToolContext) -> Result<Value> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("Pattern is required".into()))?;

    let mut forwarded = serde_json::Map::new();
    forwarded.insert("regex_pattern".to_string(), json!(pattern));
    if let Some(p) = args.get("path") {
        forwarded.insert("directory".to_string(), p.clone());
    }
    if let Some(v) = args.get("case_insensitive") {
        forwarded.insert("case_insensitive".to_string(), v.clone());
    }
    if let Some(v) = args.get("max_results") {
        forwarded.insert("max_results".to_string(), v.clone());
    }
    if let Some(v) = args.get("context_lines") {
        forwarded.insert("context_lines".to_string(), v.clone());
    }
    // `glob` (e.g. `*.rs`) didn't exist on search_files; translate to
    // `file_extension` when it's a simple `*.<ext>` pattern.
    if let Some(g) = args.get("glob").and_then(|v| v.as_str()) {
        if let Some(ext) = g.strip_prefix("*.") {
            forwarded.insert("file_extension".to_string(), json!(ext));
        }
    }

    crate::daemon::handlers::file_ops::tool_search_files(Value::Object(forwarded), ctx).await
}

// === Helpers ===

fn filter_private_items(content: &str, language: &str) -> String {
    match language {
        "rust" => content
            .lines()
            .filter(|line| {
                let t = line.trim();
                if t.starts_with("fn") || t.starts_with("struct") {
                    line.contains("pub")
                } else {
                    true
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => content.to_string(),
    }
}

fn filter_test_items(content: &str, _language: &str) -> String {
    content.to_string()
}

fn count_skeleton_items(_content: &str, _language: &str) -> serde_json::Value {
    json!({})
}

fn collect_type_names(
    function_node: &tree_sitter::Node<'_>,
    content: &str,
    lang: &str,
) -> Result<HashSet<String>> {
    use tree_sitter::Query;
    let type_query_str = match lang {
        "rust" => "(type_identifier) @type",
        "typescript" | "javascript" => "(type_identifier) @type",
        "python" => "(type) @type",
        "go" => "(type_identifier) @type",
        _ => return Ok(HashSet::new()),
    };

    let type_query = Query::new(&crate::indexer::parser::LANG_MANAGER.get_language(lang).expect("Lang not loaded").language, type_query_str)
        .map_err(|e| anyhow::anyhow!("Type query error: {}", e))?;

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&type_query, *function_node, content.as_bytes());

    let mut names = HashSet::new();
    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            if let Ok(type_name) = capture.node.utf8_text(content.as_bytes()) {
                if !is_primitive_type(type_name, lang) {
                    names.insert(type_name.to_string());
                }
            }
        }
    }
    Ok(names)
}

async fn resolve_types(
    type_names: HashSet<String>,
    sqlite: &SqliteStorage,
    file_path: &str,
) -> Result<Vec<String>> {
    if type_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut type_defs = Vec::new();
    for type_name in type_names {
        let symbols = sqlite.get_symbol_by_name(&type_name).await?;
        for symbol in symbols {
            if let Ok(Some(file_info)) = sqlite.get_file_by_id(symbol.file_id).await {
                if file_info.path == file_path
                    && (symbol.kind == crate::models::chunk::SymbolKind::Struct
                        || symbol.kind == crate::models::chunk::SymbolKind::Enum
                        || symbol.kind == crate::models::chunk::SymbolKind::Interface
                        || symbol.kind == crate::models::chunk::SymbolKind::TypeAlias)
                {
                    let type_file_path = std::path::PathBuf::from(&file_info.path);
                    if let Ok(file_content) = tokio::fs::read_to_string(&type_file_path).await {
                        let lines: Vec<&str> = file_content.lines().collect();
                        if symbol.line_start > 0 && symbol.line_end as usize <= lines.len() {
                            let start_idx = (symbol.line_start - 1) as usize;
                            let end_idx = symbol.line_end as usize;
                            let type_code = lines[start_idx..end_idx].join("\n");
                            let line_count = end_idx - start_idx;

                            type_defs.push(format!(
                                "{} ({:?} in {}, {} lines):\n{}",
                                type_name,
                                symbol.kind,
                                std::path::Path::new(&file_info.path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(""),
                                line_count,
                                type_code
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(type_defs)
}

fn collect_callee_names(
    function_node: &tree_sitter::Node<'_>,
    content: &str,
    lang: &str,
) -> Result<HashSet<String>> {
    use tree_sitter::Query;
    let call_query_str = match lang {
        "rust" => "(call_expression function: (identifier) @callee)",
        "typescript" | "javascript" => {
            "(call_expression function: (identifier) @callee)"
        }
        "python" => "(call function: (identifier) @callee)",
        "go" => "(call_expression function: (identifier) @callee)",
        _ => return Ok(HashSet::new()),
    };

    let call_query = Query::new(&crate::indexer::parser::LANG_MANAGER.get_language(lang).expect("Lang not loaded").language, call_query_str)
        .map_err(|e| anyhow::anyhow!("Call query error: {}", e))?;

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&call_query, *function_node, content.as_bytes());

    let mut names = HashSet::new();
    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            if let Ok(callee_name) = capture.node.utf8_text(content.as_bytes()) {
                names.insert(callee_name.to_string());
            }
        }
    }
    Ok(names)
}

async fn resolve_callees(
    callee_names: HashSet<String>,
    content: &str,
    lang: &str,
) -> Result<Vec<String>> {
    if callee_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut callees = Vec::new();
    for callee_name in callee_names {
        if let Some(func_info) = find_function_in_file(&callee_name, content, lang)? {
            let line_count = func_info.code.lines().count();
            callees.push(format!(
                "{} (lines {}-{}, {} lines):\n{}",
                callee_name, func_info.start_line, func_info.end_line, line_count, func_info.code
            ));
        }
    }
    Ok(callees)
}

fn is_primitive_type(type_name: &str, lang: &str) -> bool {
    match lang {
        "rust" => {
            matches!(
                type_name,
                "i8" | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "f32"
                    | "f64"
                    | "bool"
                    | "char"
                    | "str"
                    | "String"
                    | "Option"
                    | "Result"
                    | "Vec"
                    | "Box"
                    | "Arc"
                    | "Rc"
            )
        }
        "typescript" | "javascript" => {
            matches!(
                type_name,
                "string"
                    | "number"
                    | "boolean"
                    | "any"
                    | "void"
                    | "null"
                    | "undefined"
                    | "never"
                    | "unknown"
                    | "object"
                    | "Array"
                    | "Promise"
                    | "Map"
                    | "Set"
            )
        }
        "python" => {
            matches!(
                type_name,
                "int"
                    | "float"
                    | "str"
                    | "bool"
                    | "list"
                    | "dict"
                    | "tuple"
                    | "set"
                    | "None"
                    | "Any"
                    | "Optional"
                    | "Union"
            )
        }
        "go" => {
            matches!(
                type_name,
                "int"
                    | "int8"
                    | "int16"
                    | "int32"
                    | "int64"
                    | "uint"
                    | "uint8"
                    | "uint16"
                    | "uint32"
                    | "uint64"
                    | "float32"
                    | "float64"
                    | "bool"
                    | "string"
                    | "byte"
                    | "rune"
                    | "error"
                    | "interface"
            )
        }
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    code: String,
    start_line: usize,
    end_line: usize,
}

fn find_function_in_file(
    function_name: &str,
    content: &str,
    lang: &str,
) -> Result<Option<FunctionInfo>> {
    use tree_sitter::{Query, QueryCursor};

    let query_str = match lang {
        "rust" => format!(
            r#"(function_item name: (identifier) @name (#eq? @name "{}")) @func"#,
            function_name
        ),
        "typescript" | "javascript" => format!(
            r#"(function_declaration name: (identifier) @name (#eq? @name "{}")) @func"#,
            function_name
        ),
        "python" => format!(
            r#"(function_definition name: (identifier) @name (#eq? @name "{}")) @func"#,
            function_name
        ),
        "go" => format!(
            r#"(function_declaration name: (identifier) @name (#eq? @name "{}")) @func"#,
            function_name
        ),
        _ => return Ok(None),
    };

    let language = crate::indexer::parser::LANG_MANAGER.get_language(lang).expect("Lang not loaded").language.clone();
    
    let tree_opt = crate::indexer::parser::with_parser(|parser| {
        if parser.set_language(&language).is_err() {
            return None;
        }
        parser.parse(content, None)
    });

    let tree = tree_opt.ok_or_else(|| anyhow::anyhow!("Failed to parse file"))?;

    let query = Query::new(&language, &query_str)
        .map_err(|e| anyhow::anyhow!("Query error: {}", e))?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            if capture.index == 1 {
                // Assuming @func is 1
                let node = capture.node;
                if let Ok(code) = node.utf8_text(content.as_bytes()) {
                    return Ok(Some(FunctionInfo {
                        code: code.to_string(),
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                    }));
                }
            }
        }
    }

    Ok(None)
}
