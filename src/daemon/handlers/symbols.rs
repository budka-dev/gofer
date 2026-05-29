use super::common::{make_relative, resolve_path, ToolContext};
use crate::error::GoferError;
use crate::models::chunk::SymbolWithPath;
use anyhow::Result;
use serde_json::{json, Value};
use sqlx::Row;

pub async fn tool_get_symbols(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_filter = args.get("file").and_then(|v| v.as_str());
    let kind_filter = args.get("kind").and_then(|v| v.as_str());
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(500) as u32;

    // Feature 008 - Check rkyv cache first
    let cache_key = format!(
        "rkyv:{}:{}:{}:{}",
        file_filter.unwrap_or("_all_"),
        kind_filter.unwrap_or("_all_"),
        offset,
        limit
    );

    if let Some(cached_bytes) = ctx.cache.get_symbols_rkyv(&cache_key).await {
        // Zero-copy deserialization
        match rkyv::check_archived_root::<Vec<SymbolWithPath>>(&cached_bytes) {
            Ok(archived) => {
                // Convert archived data back to JSON
                let symbols: Vec<SymbolWithPath> = archived
                    .iter()
                    .map(|s| {
                        // Deserialize ArchivedSymbolKind to SymbolKind using trait method
                        use rkyv::Deserialize;
                        let kind = match s.kind.deserialize(&mut rkyv::Infallible) {
                            Ok(k) => k,
                            Err(_) => unreachable!("Infallible deserialization"),
                        };

                        SymbolWithPath {
                            id: s.id,
                            name: s.name.to_string(),
                            kind,
                            line: s.line,
                            end_line: s.end_line,
                            signature: s.signature.as_ref().map(|s| s.to_string()),
                            file_path: s.file_path.to_string(),
                        }
                    })
                    .collect();

                let count = symbols.len() as u32;

                let mut symbols_map: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for sym in &symbols {
                    let path = make_relative(&ctx.root_path, &sym.file_path);
                    let sig_str = sym.signature.as_deref().unwrap_or("");
                    let entry = if sig_str.is_empty() {
                        format!("{}: {:?} '{}'", sym.line, sym.kind, sym.name)
                    } else {
                        format!("{}: {:?} '{}' ({})", sym.line, sym.kind, sym.name, sig_str)
                    };
                    symbols_map.entry(path).or_default().push(entry);
                }

                let final_result = json!({
                    "total": count,
                    "offset": offset,
                    "limit": limit,
                    "has_more": count == limit,
                    "symbols": symbols_map
                });

                return Ok(final_result);
            }
            Err(_) => {
                // Cache corrupted, continue to fetch fresh data
            }
        }
    }

    let resolved_path = file_filter.map(|f| resolve_path(&ctx.root_path, f));
    let file_filter_resolved = resolved_path.as_deref();

    let symbols = &ctx
        .sqlite
        .get_symbols(file_filter_resolved, kind_filter, offset, limit)
        .await?;
    let count = symbols.len() as u32;

    let mut symbols_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for sym in symbols {
        let path = make_relative(&ctx.root_path, &sym.file_path);
        let sig_str = sym.signature.as_deref().unwrap_or("");
        let entry = if sig_str.is_empty() {
            format!("{}: {:?} '{}'", sym.line, sym.kind, sym.name)
        } else {
            format!("{}: {:?} '{}' ({})", sym.line, sym.kind, sym.name, sig_str)
        };
        symbols_map.entry(path).or_default().push(entry);
    }

    let final_result = json!({
        "total": count,
        "offset": offset,
        "limit": limit,
        "has_more": count == limit,
        "symbols": symbols_map
    });

    // Store in rkyv cache
    if let Ok(bytes) = rkyv::to_bytes::<_, 256>(symbols) {
        ctx.cache.put_symbols_rkyv(cache_key, bytes).await;
    }

    Ok(final_result)
}

pub async fn tool_get_references(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    let refs = &ctx.sqlite.get_references_by_name(symbol).await?;

    Ok(json!({
        "symbol": symbol,
        "total": refs.len(),
        "references": refs.iter().map(|r| {
            format!("{}:{} ({})", make_relative(&ctx.root_path, &r.file_path), r.line, r.ref_kind)
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_search_symbols(args: Value, ctx: &ToolContext) -> Result<Value> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .map(crate::models::chunk::SymbolKind::from_str);
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    if query.is_empty() {
        return Err(GoferError::InvalidParams("Query is required".into()).into());
    }

    // Using FTS search with path info
    let symbols = ctx
        .sqlite
        .search_symbols_with_path(query, (limit * 2) as i32)
        .await?;

    let mut filtered: Vec<_> = if let Some(k) = kind {
        symbols.into_iter().filter(|s| s.kind == k).collect()
    } else {
        symbols
    };
    filtered.truncate(limit);

    let mut symbols_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for sym in &filtered {
        let path = make_relative(&ctx.root_path, &sym.file_path);
        let sig_str = sym.signature.as_deref().unwrap_or("");
        let entry = if sig_str.is_empty() {
            format!("{}: {:?} '{}'", sym.line, sym.kind, sym.name)
        } else {
            format!("{}: {:?} '{}' ({})", sym.line, sym.kind, sym.name, sig_str)
        };
        symbols_map.entry(path).or_default().push(entry);
    }

    Ok(json!({
        "query": query,
        "total": filtered.len(),
        "symbols": symbols_map
    }))
}

pub async fn tool_get_callers(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    let refs = &ctx.sqlite.get_references_by_name(symbol).await?;

    // Filter for calls/usages
    let callers: Vec<_> = refs
        .iter()
        .filter(|r| r.ref_kind == "call" || r.ref_kind == "usage")
        .collect();

    Ok(json!({
        "symbol": symbol,
        "total": callers.len(),
        "callers": callers.iter().map(|r| {
            format!("{}:{} ({})", make_relative(&ctx.root_path, &r.file_path), r.line, r.ref_kind)
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_get_callees(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    let file = args.get("file").and_then(|v| v.as_str());

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    // Locate the source symbol — prefer the one in the requested file when given,
    // otherwise fall through to a global match (returning the first hit).
    let source = if let Some(f) = file {
        let abs_path = resolve_path(&ctx.root_path, f);
        ctx.sqlite
            .find_symbol_by_name_and_file(symbol, &abs_path)
            .await?
    } else {
        ctx.sqlite
            .search_symbols(symbol, 1)
            .await?
            .into_iter()
            .next()
    };

    let source = match source {
        Some(s) => s,
        None => {
            return Ok(json!({
                "symbol": symbol,
                "file": file,
                "total": 0,
                "callees": [],
                "message": "Source symbol not found in index"
            }));
        }
    };

    let refs = ctx.sqlite.get_outgoing_references(source.id).await?;
    let calls: Vec<&crate::models::SymbolReference> =
        refs.iter().filter(|r| r.kind == "call").collect();

    Ok(json!({
        "symbol": symbol,
        "file": file,
        "total": calls.len(),
        "callees": calls.iter().map(|r| {
            json!({
                "name": r.target_name,
                "line": r.line,
                "resolved": r.target_symbol_id.is_some(),
            })
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_symbol_exists(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    let file = args.get("file").and_then(|v| v.as_str());

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    let exists = if let Some(f) = file {
        let abs_path = resolve_path(&ctx.root_path, f);
        // Check if symbol exists in specific file
        let symbols = ctx
            .sqlite
            .get_symbols(Some(&abs_path), None, 0, 1000)
            .await?;
        symbols.iter().any(|s| s.name == symbol)
    } else {
        // Global check
        let matches = ctx.sqlite.search_symbols(symbol, 1).await?;
        !matches.is_empty()
    };

    Ok(json!({
        "symbol": symbol,
        "exists": exists
    }))
}

pub async fn tool_is_exported(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    let file = args.get("file").and_then(|v| v.as_str());

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    let matches = if let Some(f) = file {
        let abs_path = resolve_path(&ctx.root_path, f);
        ctx.sqlite
            .get_symbols(Some(&abs_path), None, 0, 1000)
            .await?
            .into_iter()
            .filter(|s| s.name == symbol)
            .collect()
    } else {
        ctx.sqlite.search_symbols_with_path(symbol, 10).await?
    };

    if matches.is_empty() {
        return Ok(json!({
            "symbol": symbol,
            "is_exported": false,
            "message": "Symbol not found"
        }));
    }

    // Heuristic check for export status based on signature/kind
    let is_exported = matches.iter().any(|s| {
        let sig = s.signature.as_deref().unwrap_or("").trim();
        sig.starts_with("pub ")
            || sig.starts_with("export ")
            || (!s.name.starts_with('_') && s.kind != crate::models::chunk::SymbolKind::LocalVar)
    });

    Ok(json!({
        "symbol": symbol,
        "is_exported": is_exported,
        "locations": matches.iter().map(|s| make_relative(&ctx.root_path, &s.file_path)).collect::<Vec<_>>()
    }))
}

/// Find symbols that have no incoming references in the project.
///
/// A symbol is considered "unused" when there is no entry in `symbol_references`
/// where either `target_symbol_id` equals its id, or `target_name` matches its
/// name (the latter covers unresolved references — see resolve_references).
///
/// Caveats:
/// - Public API surface that's consumed from outside the project will appear
///   unused. Filter with `file` / `kind` and use `public_only=false` accordingly.
/// - Entry points (`fn main`, `if __name__ == "__main__"`, `export default`) are
///   excluded by name+signature heuristics when `exclude_entry_points=true`.
/// - Test functions are excluded by (a) path heuristics (`tests/`, `*.test.*`,
///   …) and (b) signature attributes (`#[test]`, `#[bench]`, `#[cfg(test)]`,
///   `@pytest.fixture`). Attribute-based detection requires re-indexed data —
///   signatures only include Rust attributes after parser/core.rs's sibling
///   walk added in the same PR as this tool. Old indexes need a `force_reindex`.
/// - Export markers (`#[no_mangle]`, `extern "C"`, `#[wasm_bindgen]`,
///   `#[pyfunction]`, `#[napi]`) are excluded when `exclude_exports=true` —
///   these symbols are consumed from outside the Rust/Python world and never
///   show up in `symbol_references`.
/// - Trait/method dispatch through dyn isn't tracked — likely false positives
///   for trait impl methods.
pub async fn tool_find_unused_symbols(args: Value, ctx: &ToolContext) -> Result<Value> {
    let kind_filter = args.get("kind").and_then(|v| v.as_str());
    let file_filter = args.get("file").and_then(|v| v.as_str());
    let public_only = args
        .get("public_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let exclude_tests = args
        .get("exclude_tests")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let exclude_entry_points = args
        .get("exclude_entry_points")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let exclude_exports = args
        .get("exclude_exports")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(500) as i64;

    // Symbol kinds that meaningfully participate in the "used / unused" check.
    // `impl` blocks reference types, not the other way around; `module` doesn't
    // have call-site references; `local_var` is irrelevant for dead code.
    const DEFAULT_KINDS: &[&str] = &[
        "function",
        "method",
        "struct",
        "enum",
        "trait",
        "interface",
        "class",
        "const",
        "type",
        "type_alias",
    ];

    // Only `call`/`usage`/`inherit` references count as actual consumption.
    // Plain `import` of a module path doesn't prove the symbol is reached.
    let mut sql = String::from(
        r#"
        SELECT
            s.id          AS id,
            s.name        AS name,
            s.kind        AS kind,
            s.line_start  AS line_start,
            s.line_end    AS line_end,
            s.signature   AS signature,
            f.path        AS file_path
        FROM symbols s
        JOIN files f ON f.id = s.file_id
        WHERE NOT EXISTS (
            SELECT 1 FROM symbol_references sr
            WHERE (sr.target_symbol_id = s.id
                   OR (sr.target_symbol_id IS NULL AND sr.target_name = s.name))
              AND sr.kind IN ('call', 'usage', 'inherit', 'type_usage')
        )
        "#,
    );

    if kind_filter.is_some() {
        sql.push_str(" AND s.kind = ?");
    } else {
        sql.push_str(" AND s.kind IN (");
        for (i, _) in DEFAULT_KINDS.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push('?');
        }
        sql.push(')');
    }

    if file_filter.is_some() {
        sql.push_str(" AND f.path LIKE ?");
    }

    if exclude_tests {
        sql.push_str(
            r#"
            -- Path-based test detection (filename / directory conventions)
            AND f.path NOT LIKE '%/tests/%'
            AND f.path NOT LIKE '%/test/%'
            AND f.path NOT LIKE '%/__tests__/%'
            AND f.path NOT LIKE '%/benches/%'
            AND f.path NOT LIKE '%.test.%'
            AND f.path NOT LIKE '%.spec.%'
            AND f.path NOT LIKE '%_test.%'
            -- Signature-based test/bench attribute detection (requires
            -- re-indexed data — parser/core.rs walks attribute_item siblings).
            -- Substring match against the collapsed signature is intentionally
            -- forgiving: `#[test]`, `#[tokio::test]`, `# [test]`, etc. all hit.
            AND (s.signature IS NULL OR (
                s.signature NOT LIKE '%#[test]%'
                AND s.signature NOT LIKE '%#[tokio::test]%'
                AND s.signature NOT LIKE '%#[async_std::test]%'
                AND s.signature NOT LIKE '%#[bench]%'
                AND s.signature NOT LIKE '%#[cfg(test)]%'
                AND s.signature NOT LIKE '%#[ignore]%'
                AND s.signature NOT LIKE '%@pytest.fixture%'
                AND s.signature NOT LIKE '%@pytest.mark%'
            ))
            -- Test function naming conventions (covers Rust test_ helpers and
            -- Python test_* / *_test functions even without attributes).
            AND s.name NOT LIKE 'test\_%' ESCAPE '\'
            AND s.name NOT LIKE '%\_test' ESCAPE '\'
            "#,
        );
    }

    if exclude_entry_points {
        sql.push_str(
            r#"
            -- Common entry-point names across languages. `main` covers Rust,
            -- Go, C-like; `__main__`/`run` show up in Python/Bash; `default`
            -- catches `export default function default(...)` patterns.
            AND s.name NOT IN ('main', '__main__', 'lambda_handler', 'handler')
            "#,
        );
    }

    if exclude_exports {
        sql.push_str(
            r#"
            -- External consumers — these symbols cross the project boundary
            -- (FFI, wasm, Python ext, Node addon, etc.) so their callers live
            -- outside the indexed code.
            AND (s.signature IS NULL OR (
                s.signature NOT LIKE '%#[no_mangle]%'
                AND s.signature NOT LIKE '%extern "C"%'
                AND s.signature NOT LIKE '%extern "system"%'
                AND s.signature NOT LIKE '%pub extern %'
                AND s.signature NOT LIKE '%#[wasm_bindgen]%'
                AND s.signature NOT LIKE '%#[pyfunction]%'
                AND s.signature NOT LIKE '%#[pyclass]%'
                AND s.signature NOT LIKE '%#[pymethods]%'
                AND s.signature NOT LIKE '%#[napi]%'
                AND s.signature NOT LIKE '%#[neon::%'
                AND s.signature NOT LIKE '%#[export_name%'
                AND s.signature NOT LIKE '%@customElement%'
                AND s.signature NOT LIKE '%@Component%'
            ))
            "#,
        );
    }

    if public_only {
        sql.push_str(
            r#"
            AND (
                s.signature LIKE 'pub %'
                OR s.signature LIKE 'export %'
                OR s.signature LIKE 'export default %'
                OR (s.signature IS NULL AND s.name NOT LIKE '\_%' ESCAPE '\')
            )
            "#,
        );
    }

    sql.push_str(" ORDER BY f.path, s.line_start LIMIT ?");

    let mut query = sqlx::query(&sql);

    if let Some(k) = kind_filter {
        query = query.bind(k);
    } else {
        for k in DEFAULT_KINDS {
            query = query.bind(*k);
        }
    }

    if let Some(f) = file_filter {
        query = query.bind(format!("%{}%", f));
    }

    query = query.bind(limit + 1);

    let rows = query
        .fetch_all(ctx.sqlite.pool())
        .await
        .map_err(|e| GoferError::ToolError(format!("query failed: {}", e)))?;

    let truncated = rows.len() as i64 > limit;
    let take = rows.len().min(limit as usize);

    let mut by_kind: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut entries = Vec::with_capacity(take);

    for row in rows.iter().take(take) {
        let name: String = row.try_get("name").unwrap_or_default();
        let kind: String = row.try_get("kind").unwrap_or_default();
        let line_start: i64 = row.try_get("line_start").unwrap_or(0);
        let line_end: i64 = row.try_get("line_end").unwrap_or(0);
        let signature: Option<String> = row.try_get("signature").ok();
        let file_path: String = row.try_get("file_path").unwrap_or_default();
        let rel_path = make_relative(&ctx.root_path, &file_path);

        *by_kind.entry(kind.clone()).or_insert(0) += 1;

        entries.push(json!({
            "name": name,
            "kind": kind,
            "file": rel_path,
            "line_start": line_start,
            "line_end": line_end,
            "signature": signature,
        }));
    }

    Ok(json!({
        "count": entries.len(),
        "truncated": truncated,
        "limit": limit,
        "by_kind": by_kind,
        "filters": {
            "kind": kind_filter,
            "file": file_filter,
            "public_only": public_only,
            "exclude_tests": exclude_tests,
            "exclude_entry_points": exclude_entry_points,
            "exclude_exports": exclude_exports,
        },
        "ref_kinds_considered": ["call", "usage", "inherit", "type_usage"],
        "symbols": entries,
    }))
}

/// Find call paths between two symbols via BFS over `symbol_references`.
///
/// `direction="calls"` (default): paths where `from` transitively calls `to`.
/// `direction="called_by"`: paths where `to` is reached through callers of
/// `from` (incoming edges walked forward).
///
/// The returned path is always rendered as `from → ... → to` regardless of
/// direction. Unresolved references (target_symbol_id is NULL) are followed by
/// resolving target_name to candidate symbols — gives broader reach at the cost
/// of occasional false branches on common names.
///
/// Caveats:
/// - Single-path-per-target shortest BFS — alternative paths aren't enumerated.
/// - dyn/trait method dispatch isn't modelled. A path through a trait method
///   only appears if the static binding shows up in the index.
/// - Cyclic call graphs (recursion) are handled by visited-set short-circuit.
pub async fn tool_call_path(args: Value, ctx: &ToolContext) -> Result<Value> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let from_name = args
        .get("from")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("`from` is required".into()))?;
    let to_name = args
        .get("to")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("`to` is required".into()))?;
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("calls");
    let max_depth = args
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .min(30) as usize;
    let file_from = args.get("file_from").and_then(|v| v.as_str());
    let file_to = args.get("file_to").and_then(|v| v.as_str());
    let max_paths = args
        .get("max_paths")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(50) as usize;

    if !matches!(direction, "calls" | "called_by") {
        return Err(GoferError::InvalidParams(format!(
            "direction must be 'calls' or 'called_by', got '{}'",
            direction
        ))
        .into());
    }

    // Resolve `from` and `to` to candidate symbol ids.
    let from_candidates = resolve_symbol_candidates(ctx, from_name, file_from).await?;
    let to_candidates = resolve_symbol_candidates(ctx, to_name, file_to).await?;

    if from_candidates.is_empty() {
        return Ok(json!({
            "paths": [],
            "found": false,
            "reason": format!("`from` symbol '{}' not found in index", from_name),
        }));
    }
    if to_candidates.is_empty() {
        return Ok(json!({
            "paths": [],
            "found": false,
            "reason": format!("`to` symbol '{}' not found in index", to_name),
        }));
    }

    let start_ids: HashSet<i64> = if direction == "calls" {
        from_candidates.iter().map(|s| s.id).collect()
    } else {
        to_candidates.iter().map(|s| s.id).collect()
    };
    let target_ids: HashSet<i64> = if direction == "calls" {
        to_candidates.iter().map(|s| s.id).collect()
    } else {
        from_candidates.iter().map(|s| s.id).collect()
    };

    // BFS — record (parent, ref_kind, ref_line) for each visited node so we
    // can reconstruct the path on hit.
    let mut visited: HashSet<i64> = HashSet::new();
    let mut parent: HashMap<i64, (i64, String, i32)> = HashMap::new();
    let mut queue: VecDeque<(i64, usize)> = VecDeque::new();

    for &id in &start_ids {
        visited.insert(id);
        queue.push_back((id, 0));
    }

    let mut found_targets: Vec<i64> = Vec::new();
    while let Some((current, depth)) = queue.pop_front() {
        if target_ids.contains(&current) && !start_ids.contains(&current) {
            found_targets.push(current);
            if found_targets.len() >= max_paths {
                break;
            }
            continue;
        }
        if depth >= max_depth {
            continue;
        }

        // Edges: in "calls" direction we walk outgoing references; in
        // "called_by" we walk incoming references (callers of current).
        let edges: Vec<(i64, String, i32)> = if direction == "calls" {
            let outgoing = ctx
                .sqlite
                .get_outgoing_references(current)
                .await
                .map_err(|e| GoferError::ToolError(format!("outgoing query: {}", e)))?;
            let mut out = Vec::with_capacity(outgoing.len());
            for r in outgoing {
                if let Some(tid) = r.target_symbol_id {
                    out.push((tid, r.kind, r.line));
                } else {
                    // Resolve by name — bound the fan-out to avoid blowing up
                    // on common names like `new`.
                    let resolved = ctx
                        .sqlite
                        .get_symbol_by_name(&r.target_name)
                        .await
                        .unwrap_or_default();
                    for s in resolved.into_iter().take(16) {
                        out.push((s.id, r.kind.clone(), r.line));
                    }
                }
            }
            out
        } else {
            // called_by: current is the *target*; find sources that reference it.
            let cur_sym = ctx
                .sqlite
                .get_symbol_by_id(current)
                .await
                .ok()
                .flatten();
            let Some(sym) = cur_sym else { continue };
            let incoming = ctx
                .sqlite
                .get_incoming_references(&sym.name)
                .await
                .map_err(|e| GoferError::ToolError(format!("incoming query: {}", e)))?;
            incoming
                .into_iter()
                .map(|r| (r.source_symbol_id, r.kind, r.line))
                .collect()
        };

        for (next_id, ref_kind, ref_line) in edges {
            if !visited.contains(&next_id) {
                visited.insert(next_id);
                parent.insert(next_id, (current, ref_kind, ref_line));
                queue.push_back((next_id, depth + 1));
            }
        }
    }

    // Reconstruct paths.
    let mut paths_json: Vec<Value> = Vec::new();
    for target in &found_targets {
        let mut chain: Vec<(i64, Option<(String, i32)>)> = Vec::new();
        let mut cur = *target;
        loop {
            let edge = parent.get(&cur).map(|(_, k, l)| (k.clone(), *l));
            chain.push((cur, edge));
            if start_ids.contains(&cur) {
                break;
            }
            match parent.get(&cur) {
                Some((p, _, _)) => cur = *p,
                None => break,
            }
        }

        // chain is target → ... → start. For "calls" we want start → ... →
        // target ("from → to"). For "called_by" the natural reading is also
        // from → to in user terms — same reverse.
        chain.reverse();

        let mut nodes: Vec<Value> = Vec::with_capacity(chain.len());
        for (sid, edge) in &chain {
            let sym = ctx
                .sqlite
                .get_symbol_by_id(*sid)
                .await
                .ok()
                .flatten();
            let Some(sym) = sym else { continue };
            let file = ctx
                .sqlite
                .get_file_by_id(sym.file_id)
                .await
                .ok()
                .flatten()
                .map(|f| make_relative(&ctx.root_path, &f.path))
                .unwrap_or_default();
            let mut node = json!({
                "symbol": sym.name,
                "kind": sym.kind.as_str(),
                "file": file,
                "line": sym.line_start,
            });
            if let Some((kind, line)) = edge {
                node["incoming_edge"] = json!({ "ref_kind": kind, "at_line": line });
            }
            nodes.push(node);
        }

        paths_json.push(json!({
            "length": nodes.len().saturating_sub(1),
            "nodes": nodes,
        }));
    }

    let truncated = paths_json.len() >= max_paths;

    Ok(json!({
        "found": !paths_json.is_empty(),
        "direction": direction,
        "from": from_name,
        "to": to_name,
        "shortest_length": paths_json
            .iter()
            .filter_map(|p| p.get("length").and_then(|v| v.as_u64()))
            .min(),
        "paths": paths_json,
        "truncated": truncated,
        "max_depth": max_depth,
        "max_paths": max_paths,
    }))
}

/// Resolve a name (and optional file disambiguation) to candidate symbol rows.
async fn resolve_symbol_candidates(
    ctx: &ToolContext,
    name: &str,
    file: Option<&str>,
) -> Result<Vec<crate::models::chunk::Symbol>> {
    let all = ctx
        .sqlite
        .get_symbol_by_name(name)
        .await
        .map_err(|e| GoferError::ToolError(format!("symbol lookup: {}", e)))?;

    if let Some(f) = file {
        let abs = resolve_path(&ctx.root_path, f);
        let mut out = Vec::new();
        for sym in all {
            if let Ok(Some(file_row)) = ctx.sqlite.get_file_by_id(sym.file_id).await {
                if file_row.path == abs {
                    out.push(sym);
                }
            }
        }
        Ok(out)
    } else {
        Ok(all)
    }
}

/// Find functions/methods by type signature: return type and/or parameter type.
///
/// Matching is region-aware: `returns` is checked against the return-type
/// region of the signature, `param_type` against the parameter region — so a
/// query for `returns: Connection` won't false-match a function that merely
/// *takes* a Connection. `signature_contains` is a raw substring over the whole
/// signature for everything else.
///
/// All matches are case-sensitive substrings, so `Result<MyType` matches
/// `Result<MyType, Error>` and `&mut Conn` matches `&mut Connection`.
///
/// Caveats:
/// - Go methods with receivers (`func (s *S) Name(...)`) put the receiver in
///   the parameter region (it's the first paren group). Acceptable for
///   param_type queries; harmless for returns.
/// - Rust where-clauses bleed into the return region (`-> R where T: Clone`).
///   Substring matching still works.
/// - Arrow functions / lambdas may not split cleanly — prefer
///   `signature_contains` for those.
/// - Requires re-indexed data for accurate signatures.
pub async fn tool_find_by_type_signature(args: Value, ctx: &ToolContext) -> Result<Value> {
    let returns = args.get("returns").and_then(|v| v.as_str());
    let param_type = args.get("param_type").and_then(|v| v.as_str());
    let signature_contains = args.get("signature_contains").and_then(|v| v.as_str());
    let kind_filter = args.get("kind").and_then(|v| v.as_str());
    let file_filter = args.get("file").and_then(|v| v.as_str());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(500) as usize;

    if returns.is_none() && param_type.is_none() && signature_contains.is_none() {
        return Err(GoferError::InvalidParams(
            "Provide at least one of: `returns`, `param_type`, `signature_contains`".into(),
        )
        .into());
    }

    // Coarse SQL pre-filter: every needle must appear *somewhere* in the
    // signature. Region-aware refinement happens in Rust below.
    let mut sql = String::from(
        r#"
        SELECT s.name AS name, s.kind AS kind, s.signature AS signature,
               s.line_start AS line_start, f.path AS file_path
        FROM symbols s
        JOIN files f ON f.id = s.file_id
        WHERE s.signature IS NOT NULL
        "#,
    );

    if kind_filter.is_some() {
        sql.push_str(" AND s.kind = ?");
    } else {
        sql.push_str(" AND s.kind IN ('function', 'method')");
    }
    if file_filter.is_some() {
        sql.push_str(" AND f.path LIKE ?");
    }
    for _ in [returns, param_type, signature_contains].iter().filter(|x| x.is_some()) {
        sql.push_str(" AND s.signature LIKE ?");
    }
    sql.push_str(" ORDER BY f.path, s.line_start LIMIT 5000");

    let mut query = sqlx::query(&sql);
    if let Some(k) = kind_filter {
        query = query.bind(k);
    }
    if let Some(f) = file_filter {
        query = query.bind(format!("%{}%", f));
    }
    for needle in [returns, param_type, signature_contains].into_iter().flatten() {
        query = query.bind(format!("%{}%", needle));
    }

    let rows = query
        .fetch_all(ctx.sqlite.pool())
        .await
        .map_err(|e| GoferError::ToolError(format!("query failed: {}", e)))?;

    let mut matches = Vec::new();
    let mut by_kind: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for row in &rows {
        let sig: String = row.try_get("signature").unwrap_or_default();
        let (params_region, return_region) = split_signature(&sig);

        if let Some(r) = returns {
            if !return_region.contains(r) {
                continue;
            }
        }
        if let Some(p) = param_type {
            if !params_region.contains(p) {
                continue;
            }
        }
        // signature_contains already enforced by SQL LIKE; nothing to refine.

        let name: String = row.try_get("name").unwrap_or_default();
        let kind: String = row.try_get("kind").unwrap_or_default();
        let line_start: i64 = row.try_get("line_start").unwrap_or(0);
        let file_path: String = row.try_get("file_path").unwrap_or_default();
        let rel = make_relative(&ctx.root_path, &file_path);

        *by_kind.entry(kind.clone()).or_insert(0) += 1;
        matches.push(json!({
            "name": name,
            "kind": kind,
            "file": rel,
            "line": line_start,
            "signature": sig,
        }));
        if matches.len() >= limit {
            break;
        }
    }

    Ok(json!({
        "count": matches.len(),
        "truncated": matches.len() >= limit,
        "by_kind": by_kind,
        "filters": {
            "returns": returns,
            "param_type": param_type,
            "signature_contains": signature_contains,
            "kind": kind_filter,
            "file": file_filter,
        },
        "matches": matches,
    }))
}

/// Split a collapsed signature into (params_region, return_region).
///
/// Params = the first top-level `(...)` group. Return = the type after it,
/// recognising `->` (Rust/Python), leading `:` (TS), or a bare trailing type
/// (Go funcs). Pure string fn — unit-testable without a grammar.
fn split_signature(sig: &str) -> (String, String) {
    let open = match sig.find('(') {
        Some(i) => i,
        None => return (String::new(), String::new()),
    };

    // Find the matching close paren by depth.
    let mut depth = 0i32;
    let mut close = None;
    for (i, ch) in sig[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = match close {
        Some(c) => c,
        None => return (sig[open + 1..].trim().to_string(), String::new()),
    };

    let params = sig[open + 1..close].trim().to_string();
    let rest = sig[close + 1..].trim();

    let ret_raw = if let Some(idx) = rest.find("->") {
        rest[idx + 2..].trim()
    } else if let Some(stripped) = rest.strip_prefix(':') {
        stripped.trim()
    } else {
        rest
    };
    let ret = ret_raw
        .trim_end_matches(|c| c == '{' || c == ':' || c == ';' || c == ' ')
        .trim()
        .to_string();

    (params, ret)
}

/// Build the dependency subgraph around a symbol via BFS over
/// `symbol_references`, bounded by depth and node count.
///
/// `direction`:
/// - `out` (default): what the symbol depends on (outgoing edges, transitively)
/// - `in`: what depends on the symbol (incoming edges)
/// - `both`: union of the two
///
/// Returns the set of reachable nodes and the edges between them — a focused
/// neighbourhood, not a path to a specific target (use `call_path` for that).
pub async fn tool_dependency_subgraph(args: Value, ctx: &ToolContext) -> Result<Value> {
    use std::collections::{HashSet, VecDeque};

    let symbol = args
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("`symbol` is required".into()))?;
    let file = args.get("file").and_then(|v| v.as_str());
    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("out");
    let max_depth = args
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(3)
        .min(20) as usize;
    let max_nodes = args
        .get("max_nodes")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(500) as usize;

    if !matches!(direction, "out" | "in" | "both") {
        return Err(GoferError::InvalidParams(format!(
            "direction must be 'out', 'in' or 'both', got '{}'",
            direction
        ))
        .into());
    }

    let roots = resolve_symbol_candidates(ctx, symbol, file).await?;
    if roots.is_empty() {
        return Ok(json!({
            "found": false,
            "reason": format!("symbol '{}' not found in index", symbol),
            "nodes": [],
            "edges": [],
        }));
    }

    let mut visited: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<(i64, usize)> = VecDeque::new();
    let mut edges: Vec<(i64, i64, String)> = Vec::new();
    let mut truncated = false;

    for r in &roots {
        if visited.insert(r.id) {
            queue.push_back((r.id, 0));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let mut neighbours: Vec<(i64, String)> = Vec::new();

        if direction == "out" || direction == "both" {
            let outgoing = ctx
                .sqlite
                .get_outgoing_references(current)
                .await
                .map_err(|e| GoferError::ToolError(format!("outgoing: {}", e)))?;
            for r in outgoing {
                if let Some(tid) = r.target_symbol_id {
                    edges.push((current, tid, r.kind.clone()));
                    neighbours.push((tid, r.kind));
                }
            }
        }
        if direction == "in" || direction == "both" {
            if let Ok(Some(sym)) = ctx.sqlite.get_symbol_by_id(current).await {
                let incoming = ctx
                    .sqlite
                    .get_incoming_references(&sym.name)
                    .await
                    .map_err(|e| GoferError::ToolError(format!("incoming: {}", e)))?;
                for r in incoming {
                    edges.push((r.source_symbol_id, current, r.kind.clone()));
                    neighbours.push((r.source_symbol_id, r.kind));
                }
            }
        }

        for (nid, _) in neighbours {
            if visited.len() >= max_nodes {
                truncated = true;
                break;
            }
            if visited.insert(nid) {
                queue.push_back((nid, depth + 1));
            }
        }
        if truncated {
            break;
        }
    }

    // Materialise nodes.
    let mut nodes_json = Vec::with_capacity(visited.len());
    for id in &visited {
        if let Ok(Some(sym)) = ctx.sqlite.get_symbol_by_id(*id).await {
            let file_path = ctx
                .sqlite
                .get_file_by_id(sym.file_id)
                .await
                .ok()
                .flatten()
                .map(|f| make_relative(&ctx.root_path, &f.path))
                .unwrap_or_default();
            nodes_json.push(json!({
                "id": id,
                "symbol": sym.name,
                "kind": sym.kind.as_str(),
                "file": file_path,
                "line": sym.line_start,
                "is_root": roots.iter().any(|r| r.id == *id),
            }));
        }
    }

    // Keep only edges whose both endpoints are in the visited set (dropped when
    // truncation cut a neighbour).
    let edges_json: Vec<Value> = edges
        .iter()
        .filter(|(f, t, _)| visited.contains(f) && visited.contains(t))
        .map(|(f, t, k)| json!({ "from": f, "to": t, "ref_kind": k }))
        .collect();

    Ok(json!({
        "found": true,
        "symbol": symbol,
        "direction": direction,
        "node_count": nodes_json.len(),
        "edge_count": edges_json.len(),
        "truncated": truncated,
        "max_depth": max_depth,
        "max_nodes": max_nodes,
        "nodes": nodes_json,
        "edges": edges_json,
    }))
}

/// Find implementations of a trait/interface/base by name, across languages.
///
/// - Rust: `impl <Name> for <Type>` blocks (extracts the trait position).
/// - TS/JS: `class X implements <Name>` / `class X extends <Name>`.
/// - Python: `class X(<Name>)` base classes.
///
/// Go is NOT supported — interface satisfaction is structural (method sets),
/// not declared, so it can't be found by signature.
pub async fn tool_find_implementations(args: Value, ctx: &ToolContext) -> Result<Value> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("`name` is required".into()))?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(500) as i64;

    // Coarse: impl/class symbols whose signature mentions the name.
    let sql = r#"
        SELECT s.name AS name, s.kind AS kind, s.signature AS signature,
               s.line_start AS line_start, f.path AS file_path
        FROM symbols s
        JOIN files f ON f.id = s.file_id
        WHERE s.kind IN ('impl', 'class', 'struct')
          AND s.signature IS NOT NULL
          AND s.signature LIKE ?
        ORDER BY f.path, s.line_start
        LIMIT 5000
    "#;

    let rows = sqlx::query(sql)
        .bind(format!("%{}%", name))
        .fetch_all(ctx.sqlite.pool())
        .await
        .map_err(|e| GoferError::ToolError(format!("query failed: {}", e)))?;

    let mut impls = Vec::new();
    for row in &rows {
        let sig: String = row.try_get("signature").unwrap_or_default();
        let kind: String = row.try_get("kind").unwrap_or_default();
        let sym_name: String = row.try_get("name").unwrap_or_default();
        let line: i64 = row.try_get("line_start").unwrap_or(0);
        let file_path: String = row.try_get("file_path").unwrap_or_default();

        let relation = classify_implementation(&sig, name);
        let Some(relation) = relation else { continue };

        impls.push(json!({
            "type": sym_name,
            "kind": kind,
            "relation": relation,
            "file": make_relative(&ctx.root_path, &file_path),
            "line": line,
            "signature": sig,
        }));
        if impls.len() as i64 >= limit {
            break;
        }
    }

    Ok(json!({
        "name": name,
        "count": impls.len(),
        "truncated": impls.len() as i64 >= limit,
        "implementations": impls,
        "note": "Go interface satisfaction is structural and not detected here.",
    }))
}

/// Classify how a signature relates to `name`. Returns the relation label
/// ("implements_trait" / "implements" / "extends" / "subclass") or None.
fn classify_implementation(sig: &str, name: &str) -> Option<String> {
    let s = sig.trim();

    // Rust: `impl <Trait> for <Type>` — the trait must contain `name`.
    if s.starts_with("impl") {
        if let Some(trait_part) = impl_trait_name(s) {
            if token_contains(trait_part, name) {
                return Some("implements_trait".to_string());
            }
        }
        return None;
    }

    // TS: `class X implements I, J` / `class X extends Base`.
    if let Some(idx) = s.find("implements") {
        let after = &s[idx + "implements".len()..];
        if token_contains(after, name) {
            return Some("implements".to_string());
        }
    }
    if let Some(idx) = s.find("extends") {
        let after = &s[idx + "extends".len()..];
        if token_contains(after, name) {
            return Some("extends".to_string());
        }
    }

    // Python: `class X(Base, Mixin)` — bases in parens.
    if let (Some(open), Some(close)) = (s.find('('), s.rfind(')')) {
        if open < close {
            let bases = &s[open + 1..close];
            if token_contains(bases, name) {
                return Some("subclass".to_string());
            }
        }
    }

    None
}

/// Extract the trait portion of a Rust impl signature: `impl Trait for Type`
/// → Some("Trait"). Skips a leading generic param group (`impl<T> ...`).
/// Returns None for inherent impls (`impl Type`).
fn impl_trait_name(sig: &str) -> Option<&str> {
    let rest = sig.strip_prefix("impl")?;
    // Skip a leading `<...>` generic-params group on the impl itself.
    let rest = rest.trim_start();
    let rest = if rest.starts_with('<') {
        let mut depth = 0i32;
        let mut end = None;
        for (i, ch) in rest.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        &rest[end?..]
    } else {
        rest
    };
    // The trait is everything up to ` for `. No ` for ` → inherent impl.
    let for_idx = rest.find(" for ")?;
    Some(rest[..for_idx].trim())
}

/// Whether `haystack` contains `name` as a whole identifier token (so `Foo`
/// doesn't match `FooBar`). Boundaries are non-identifier chars.
fn token_contains(haystack: &str, name: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(name) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !haystack[..abs]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        let after_idx = abs + name.len();
        let after_ok = after_idx >= haystack.len()
            || !haystack[after_idx..]
                .chars()
                .next()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        if before_ok && after_ok {
            return true;
        }
        start = abs + name.len();
    }
    false
}


#[cfg(test)]
mod tests {
    use super::split_signature;

    #[test]
    fn rust_return() {
        let (p, r) = split_signature("fn foo(a: A, b: B) -> Result<MyType, Error>");
        assert_eq!(p, "a: A, b: B");
        assert_eq!(r, "Result<MyType, Error>");
    }

    #[test]
    fn rust_no_return() {
        let (p, r) = split_signature("fn foo(a: A)");
        assert_eq!(p, "a: A");
        assert_eq!(r, "");
    }

    #[test]
    fn rust_ref_mut_param() {
        let (p, _) = split_signature("fn handle(conn: &mut Connection, n: usize) -> bool");
        assert!(p.contains("&mut Connection"));
    }

    #[test]
    fn rust_where_clause_bleeds_into_return() {
        let (_, r) = split_signature("fn f() -> R where T: Clone");
        // Documented behaviour: where-clause stays in the return region.
        assert!(r.starts_with("R"));
    }

    #[test]
    fn ts_return() {
        let (p, r) = split_signature("function foo(a: number): Promise<void>");
        assert_eq!(p, "a: number");
        assert_eq!(r, "Promise<void>");
    }

    #[test]
    fn python_return() {
        let (p, r) = split_signature("def foo(a, b) -> Dict:");
        assert_eq!(p, "a, b");
        assert_eq!(r, "Dict");
    }

    #[test]
    fn go_bare_return() {
        let (p, r) = split_signature("func foo(a int) error");
        assert_eq!(p, "a int");
        assert_eq!(r, "error");
    }

    #[test]
    fn go_parenthesized_return() {
        let (_, r) = split_signature("func foo(a int) (Result, error)");
        assert!(r.contains("Result") && r.contains("error"));
    }

    #[test]
    fn generics_in_params_dont_break_paren_matching() {
        let (p, r) = split_signature("fn f(cb: fn(i32) -> i32, x: i32) -> i32");
        // The nested `(i32)` inside the fn-pointer param must not terminate
        // the param region early.
        assert!(p.contains("cb: fn(i32) -> i32"));
        assert_eq!(r, "i32");
    }

    #[test]
    fn no_parens_returns_empty() {
        let (p, r) = split_signature("const X: i32");
        assert_eq!(p, "");
        assert_eq!(r, "");
    }

    // === find_implementations helpers ===
    use super::{classify_implementation, impl_trait_name, token_contains};

    #[test]
    fn impl_trait_simple() {
        assert_eq!(impl_trait_name("impl Display for Foo"), Some("Display"));
    }

    #[test]
    fn impl_trait_with_generics() {
        assert_eq!(impl_trait_name("impl<T> From<T> for Bar"), Some("From<T>"));
    }

    #[test]
    fn impl_inherent_is_none() {
        // `impl Foo` (inherent, no trait) — not an implementation of anything.
        assert_eq!(impl_trait_name("impl Foo"), None);
    }

    #[test]
    fn classify_rust_impl() {
        assert_eq!(
            classify_implementation("impl Display for Foo", "Display"),
            Some("implements_trait".to_string())
        );
        // The TYPE position must not count as implementing.
        assert_eq!(classify_implementation("impl Display for Foo", "Foo"), None);
    }

    #[test]
    fn classify_ts_implements() {
        assert_eq!(
            classify_implementation("class Widget implements Renderable", "Renderable"),
            Some("implements".to_string())
        );
    }

    #[test]
    fn classify_ts_extends() {
        assert_eq!(
            classify_implementation("class Button extends Component", "Component"),
            Some("extends".to_string())
        );
    }

    #[test]
    fn classify_python_subclass() {
        assert_eq!(
            classify_implementation("class Dog(Animal, Mixin)", "Animal"),
            Some("subclass".to_string())
        );
    }

    #[test]
    fn token_contains_respects_boundaries() {
        assert!(token_contains("implements Renderable", "Renderable"));
        // `Foo` must not match inside `FooBar`.
        assert!(!token_contains("implements FooBar", "Foo"));
        assert!(token_contains("Animal, Mixin", "Mixin"));
    }
}
