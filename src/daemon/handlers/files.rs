use super::common::ToolContext;
use crate::error::GoferError;
use crate::storage::SqliteStorage;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashSet;
use streaming_iterator::StreamingIterator;



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

    // Structured symbol list from index (same file) for agents that skip prose.
    let abs = file_path.to_string_lossy().to_string();
    let index_symbols = ctx
        .sqlite
        .get_symbols(Some(&abs), None, 0, 500)
        .await
        .unwrap_or_default();
    let symbols_json: Vec<Value> = index_symbols
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "kind": s.kind,
                "line": s.line,
                "end_line": s.end_line,
                "signature": s.signature,
            })
        })
        .collect();

    Ok(json!({
        "file": file,
        "file_path": file,
        "language": language,
        "skeleton_content": skeleton,
        "symbols": symbols_json,
        "stats": {
            "original_lines": original_lines,
            "original_chars": original_chars,
            "skeleton_lines": skeleton_lines,
            "skeleton_chars": skeleton_chars,
            "reduction_percent": format!("{:.1}", reduction_percent),
            "items_kept": items,
            "index_symbols": symbols_json.len(),
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
    let (function_code, start_line, end_line, type_names, callee_names, imports_filtered) = {
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

        let imports_filtered = if include_imports {
            let all_imports = collect_imports(&tree, &content, lang)?;
            let used_idents = collect_function_identifiers(&function_node, &content, lang)?;
            all_imports
                .into_iter()
                .filter(|(_text, names)| names.iter().any(|n| used_idents.contains(n)))
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        (
            function_code,
            start_line,
            end_line,
            type_names,
            callee_names,
            imports_filtered,
        )
    };

    // Now call async functions
    let mut types = Vec::new();
    if include_types {
        types = resolve_types(type_names, &ctx.sqlite, file).await?;
    }

    let imports: Vec<String> = imports_filtered;

    let mut callees = Vec::new();
    if include_callees {
        callees = resolve_callees(callee_names, &content, lang).await?;
    }

    Ok(json!({
        "file": file,
        "function": function,
        "language": lang,
        "code": function_code,
        "start_line": start_line,
        "end_line": end_line,
        "line_count": end_line.saturating_sub(start_line).saturating_add(1),
        "referenced_types": types,
        "imports": imports,
        "callees": callees,
        "stats": {
            "code_chars": function_code.len(),
            "types": types.len(),
            "imports": imports.len(),
            "callees": callees.len(),
        }
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
        let types: Vec<Value> = blocks
            .iter()
            .map(|b| {
                let first = b.lines().find(|l| !l.trim().starts_with("///") && !l.trim().starts_with("/*") && !l.trim().is_empty()).unwrap_or("");
                json!({ "text": b, "preview": first.trim() })
            })
            .collect();
        return Ok(json!({
            "file": file,
            "kind": kind_filter,
            "total": types.len(),
            "types": types,
            "types_content": blocks.join("\n\n"),
        }));
    }

    // Prefer index symbols for type-like kinds when available (precise lines).
    let abs = file_path.to_string_lossy().to_string();
    let mut index_types = ctx
        .sqlite
        .get_symbols(Some(&abs), None, 0, 500)
        .await
        .unwrap_or_default();
    index_types.retain(|s| {
        matches!(
            s.kind.as_str(),
            "struct" | "enum" | "interface" | "type" | "type_alias" | "class" | "trait"
        ) && kind_filter.map(|k| s.kind.as_str().contains(k) || s.name.contains(k)).unwrap_or(true)
    });
    if !index_types.is_empty() {
        let types: Vec<Value> = index_types
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "kind": s.kind,
                    "line": s.line,
                    "end_line": s.end_line,
                    "signature": s.signature,
                })
            })
            .collect();
        return Ok(json!({
            "file": file,
            "kind": kind_filter,
            "total": types.len(),
            "types": types,
            "source": "index",
        }));
    }

    // Fallback: skeleton line filter
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
        "total": filtered_lines.len(),
        "types_content": filtered_lines.join("\n"),
        "source": "skeleton",
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

fn collect_imports(
    tree: &tree_sitter::Tree,
    content: &str,
    lang: &str,
) -> Result<Vec<(String, HashSet<String>)>> {
    use tree_sitter::Query;
    // Capture each import declaration at the granularity we want to filter on.
    // For Go we use `import_spec` so multi-line `import (...)` blocks are split
    // per package; everywhere else the whole statement is one unit.
    let query_str = match lang {
        "rust" => "(use_declaration) @import",
        "typescript" | "javascript" => "(import_statement) @import",
        "python" => "[(import_statement) (import_from_statement)] @import",
        "go" => "(import_spec) @import",
        _ => return Ok(Vec::new()),
    };

    let language = &crate::indexer::parser::LANG_MANAGER
        .get_language(lang)
        .expect("Lang not loaded")
        .language;
    let query = Query::new(language, query_str)
        .map_err(|e| anyhow::anyhow!("Import query error: {}", e))?;

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

    let mut imports = Vec::new();
    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let node = capture.node;
            let text = match node.utf8_text(content.as_bytes()) {
                Ok(t) => t.to_string(),
                Err(_) => continue,
            };
            let mut names = collect_descendant_identifiers(&node, content);
            // For `import "fmt"` (Go) the package name is the basename of the
            // string literal and there are no identifier children to pick up.
            if lang == "go" {
                if let Some(path_node) = node.child_by_field_name("path") {
                    if let Ok(raw) = path_node.utf8_text(content.as_bytes()) {
                        let trimmed = raw.trim_matches('"');
                        if let Some(last) = trimmed.rsplit('/').next() {
                            if !last.is_empty() {
                                names.insert(last.to_string());
                            }
                        }
                    }
                }
            }
            imports.push((text, names));
        }
    }
    Ok(imports)
}

fn collect_function_identifiers(
    function_node: &tree_sitter::Node<'_>,
    content: &str,
    _lang: &str,
) -> Result<HashSet<String>> {
    Ok(collect_descendant_identifiers(function_node, content))
}

/// Walk a node's subtree and collect every identifier-like token. Used by both
/// import filtering (compute the set of names an import would introduce) and
/// function-identifier collection (compute names actually referenced inside).
fn collect_descendant_identifiers(node: &tree_sitter::Node<'_>, content: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = node.walk();
    let mut stack: Vec<tree_sitter::Node<'_>> = node.children(&mut cursor).collect();
    // Also consider the node itself if it's an identifier (rare but possible).
    if is_identifier_kind(node.kind()) {
        if let Ok(t) = node.utf8_text(content.as_bytes()) {
            names.insert(t.to_string());
        }
    }
    while let Some(n) = stack.pop() {
        if is_identifier_kind(n.kind()) {
            if let Ok(t) = n.utf8_text(content.as_bytes()) {
                names.insert(t.to_string());
            }
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            stack.push(child);
        }
    }
    names
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "package_identifier"
            | "namespace_identifier"
    )
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
