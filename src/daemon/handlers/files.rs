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
                r#"(function_item name: (identifier) @name (#eq? @name "{}")) @func"#,
                function
            ),
            "typescript" | "javascript" => format!(
                r#"(function_declaration name: (identifier) @name (#eq? @name "{}")) @func"#,
                function
            ),
            "python" => format!(
                r#"(function_definition name: (identifier) @name (#eq? @name "{}")) @func"#,
                function
            ),
            "go" => format!(
                r#"(function_declaration name: (identifier) @name (#eq? @name "{}")) @func"#,
                function
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

    let skeleton = crate::indexer::context::skeletonize_content(&content, ext);

    // Filter lines to keep only type definitions
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
        "types_content": filtered_lines.join("\n")
    }))
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

    let Some(pat) = pattern else {
        return Err(GoferError::InvalidParams("Pattern is required".into()).into());
    };

    let search_root = if let Some(p) = path_filter {
        ctx.root_path.join(p)
    } else {
        ctx.root_path.as_ref().clone()
    };

    // Check if search_root exists
    if !search_root.exists() {
        return Ok(json!({
            "pattern": pat,
            "count": 0,
            "files": []
        }));
    }

    let pat_string = pat.to_string();
    let ctx_root_path = ctx.root_path.clone();

    let files = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();

        // Parse glob pattern inside block
        let glob_pattern =
            glob::Pattern::new(&pat_string).map_err(|e| format!("Invalid glob pattern: {}", e))?;

        // Use WalkDir for traversal
        let walker = WalkDir::new(&search_root).into_iter();

        for entry in walker.filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }

            // Skip .git directories
            if entry.path().to_string_lossy().contains("/.git/") {
                continue;
            }

            // Get relative path from search_root for pattern matching
            let relative_to_search = entry
                .path()
                .strip_prefix(&search_root)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or("");

            // Match against glob pattern
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

    Ok(json!({
        "pattern": pat,
        "count": files.len(),
        "files": files
    }))
}

pub async fn tool_grep(args: Value, ctx: &ToolContext) -> Result<Value> {
    let pattern = args.get("pattern").and_then(|v| v.as_str());
    let path_filter = args.get("path").and_then(|v| v.as_str());
    let glob_filter = args.get("glob").and_then(|v| v.as_str());
    let case_insensitive = args
        .get("case_insensitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    let Some(pat) = pattern else {
        return Err(GoferError::InvalidParams("Pattern is required".into()).into());
    };

    use regex::RegexBuilder;

    let re = RegexBuilder::new(pat)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| GoferError::InvalidParams(format!("Invalid regex: {}", e)))?;

    let search_root = if let Some(p) = path_filter {
        ctx.root_path.join(p)
    } else {
        ctx.root_path.as_ref().clone()
    };

    let glob_filter_string = glob_filter.map(|s| s.to_string());
    let ctx_root_path = ctx.root_path.clone();

    let (file_matches, count) = tokio::task::spawn_blocking(move || {
        let mut file_matches: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut count = 0;

        let walker = WalkDir::new(&search_root).into_iter();

        let glob_pat = glob_filter_string.and_then(|g| glob::Pattern::new(&g).ok());

        for entry in walker.filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }

            if entry.path().to_string_lossy().contains("/.git/") {
                continue;
            }

            if let Some(ref gp) = glob_pat {
                if !gp.matches_path(entry.path()) {
                    continue;
                }
            }

            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                let mut file_hits = Vec::new();
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        file_hits.push(format!("{}: {}", i + 1, line.trim()));
                        count += 1;
                        if count >= max_results {
                            break;
                        }
                    }
                }
                if !file_hits.is_empty() {
                    let rel_path =
                        make_relative(&ctx_root_path, entry.path().to_str().unwrap_or(""));
                    file_matches.insert(rel_path, file_hits);
                }
            }
            if count >= max_results {
                break;
            }
        }
        Ok::<_, String>((file_matches, count))
    })
    .await
    .map_err(|e| GoferError::Internal(anyhow::anyhow!("Task panic: {}", e)))?
    .map_err(|e| GoferError::Internal(anyhow::anyhow!(e)))?;

    Ok(json!({
        "pattern": pat,
        "count": count,
        "matches": file_matches
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
