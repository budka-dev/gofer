use super::common::{make_relative, resolve_path, ToolContext};
use crate::error::GoferError;
use crate::models::chunk::SymbolWithPath;
use anyhow::Result;
use serde_json::{json, Value};

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

pub async fn tool_has_documentation(args: Value, ctx: &ToolContext) -> Result<Value> {
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
            "has_docs": false,
            "message": "Symbol not found"
        }));
    }

    Ok(json!({
        "symbol": symbol,
        "has_docs": false,
        "message": "Documentation status check requires reading file (not implemented in lightweight tool)"
    }))
}
