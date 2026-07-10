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
    let file = args.get("file").and_then(|v| v.as_str());

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    let defining_file = file.map(|f| resolve_path(&ctx.root_path, f));
    let refs = ctx
        .sqlite
        .get_references_for_symbol(symbol, defining_file.as_deref(), true)
        .await?;

    Ok(json!({
        "symbol": symbol,
        "file": file,
        "total": refs.len(),
        "precision": "prefer_resolved",
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
    let file = args.get("file").and_then(|v| v.as_str());

    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("Symbol name is required".into()).into());
    }

    let defining_file = file.map(|f| resolve_path(&ctx.root_path, f));
    let refs = ctx
        .sqlite
        .get_references_for_symbol(symbol, defining_file.as_deref(), true)
        .await?;

    // Filter for calls/usages
    let callers: Vec<_> = refs
        .iter()
        .filter(|r| r.ref_kind == "call" || r.ref_kind == "usage")
        .collect();

    Ok(json!({
        "symbol": symbol,
        "file": file,
        "total": callers.len(),
        "precision": "prefer_resolved",
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
