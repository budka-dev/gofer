use super::common::{make_relative, resolve_path, ToolContext};
use crate::error::GoferError;
use crate::models::chunk::SymbolKind;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

/// Fused search hit from vector + FTS results
pub struct FusedHit {
    pub file_path: String,
    pub line_start: u32,
    pub content: String,
    pub rrf_score: f64,
    pub vector_score: Option<f32>,
    pub matched_symbol: Option<String>,
    pub symbol_kind: Option<SymbolKind>,
}

/// Max chars of hit content in search results (token hygiene).
const SEARCH_CONTENT_MAX_CHARS: usize = 600;
/// Soft cap: keep at most this many hits per file after ranking.
const SEARCH_MAX_PER_FILE: usize = 3;

pub async fn tool_search(args: Value, ctx: &ToolContext) -> Result<Value> {
    use std::time::Instant;
    let search_start = Instant::now();

    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    // NEW: Phase 0 Feature 006 - search_with_scores
    let include_scores = args
        .get("include_scores")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let preview_mode = args
        .get("preview_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let min_score = args
        .get("min_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let include_context = args
        .get("include_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Extract path filter for use in vector and FTS search
    let path_filter = args.get("path").and_then(|v| v.as_str());
    let glob_filter = args.get("glob").and_then(|v| v.as_str());
    let max_per_file = args
        .get("max_per_file")
        .and_then(|v| v.as_u64())
        .unwrap_or(SEARCH_MAX_PER_FILE as u64)
        .clamp(1, 20) as usize;

    if query.is_empty() {
        return Err(GoferError::InvalidParams("Query is required".into()).into());
    }

    // Cache key includes filters so path/glob/preview variants don't collide.
    let cache_fingerprint = format!(
        "v2|{}|l={}|p={}|g={}|prev={}|min={:.3}|sc={}|ctx={}|mpf={}",
        query,
        limit,
        path_filter.unwrap_or(""),
        glob_filter.unwrap_or(""),
        preview_mode,
        min_score,
        include_scores,
        include_context,
        max_per_file
    );
    if let Some(cached_json) = ctx.cache.get_search(&cache_fingerprint, 0).await {
        if let Ok(cached_result) = serde_json::from_str::<Value>(&cached_json) {
            return Ok(cached_result);
        }
    }

    // Feature 016: Track warnings for degraded mode
    let mut warnings: Vec<String> = Vec::new();
    let mut degraded = false;

    // 1. Vector search (semantic) with circuit breaker
    let embedding_result = ctx
        .embedding_circuit
        .call(|| async {
            ctx.embedder
                .embed_query(query)
                .await
                .map_err(|e| anyhow::anyhow!(e))
        })
        .await;

    let path_filter_abs = path_filter.map(|p| resolve_path(&ctx.root_path, p));

    let vector_results = match embedding_result {
        Ok(embedding) => {
            // Try vector search with circuit breaker and path filter
            match ctx
                .vector_circuit
                .call(|| async {
                    ctx.lance
                        .search_with_filter(&embedding, limit * 2, path_filter_abs.as_deref())
                        .await
                        .map_err(|e| anyhow::anyhow!(e))
                })
                .await
            {
                Ok(results) => results,
                Err(e) => {
                    tracing::warn!(
                        "Vector search failed: {}, falling back to keyword search",
                        e
                    );
                    warnings.push(format!("Vector search unavailable: {}", e));
                    degraded = true;
                    Vec::new()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Embedding failed: {}, falling back to keyword search", e);
            warnings.push(format!("Embedding service unavailable: {}", e));
            degraded = true;
            Vec::new()
        }
    };

    // 2. FTS5 search (keyword) — build FTS query from words, ignore errors
    let fts_query = query
        .split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let fts_results = match ctx
        .sqlite
        .search_symbols_with_path_filter(&fts_query, (limit * 2) as i32, path_filter_abs.as_deref())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("FTS search failed (continuing with vector only): {}", e);
            Vec::new()
        }
    };

    // 3. RRF fusion (k=60)
    const K: f64 = 60.0;

    let mut scores: HashMap<(String, u32), FusedHit> = HashMap::new();

    // Query tokens for exact-name / path boosts (case-insensitive whole token).
    let query_tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .collect();

    let path_boost = |file_path: &str| -> f64 {
        let path_l = file_path.to_lowercase();
        let mut b: f64 = 0.0;
        for t in &query_tokens {
            if t.len() >= 3 && path_l.contains(t.as_str()) {
                b += 0.12;
            }
        }
        b.min(0.36_f64)
    };

    // Vector results contribute
    for (rank, hit) in vector_results.iter().enumerate() {
        let key = (hit.file_path.clone(), hit.line_start);
        let rrf = 1.0 / (K + rank as f64 + 1.0) + path_boost(&hit.file_path);
        scores
            .entry(key)
            .and_modify(|h| {
                h.rrf_score += rrf;
                if h.vector_score.is_none() {
                    h.vector_score = Some(hit.score);
                }
            })
            .or_insert(FusedHit {
                file_path: hit.file_path.clone(),
                line_start: hit.line_start,
                content: hit.content.clone(),
                rrf_score: rrf,
                vector_score: Some(hit.score),
                matched_symbol: None,
                symbol_kind: None,
            });
    }

    // Symbol FTS results contribute
    for (rank, sym) in fts_results.iter().enumerate() {
        let key = (sym.file_path.clone(), sym.line as u32);
        let mut rrf = 1.0 / (K + rank as f64 + 1.0) + path_boost(&sym.file_path);
        // Exact symbol-name match beats partial FTS noise.
        let name_l = sym.name.to_lowercase();
        if query_tokens.iter().any(|t| t == &name_l) {
            rrf += 0.5; // strong boost into RRF fusion
        }
        let content = sym.signature.as_deref().unwrap_or(&sym.name).to_string();
        scores
            .entry(key)
            .and_modify(|h| {
                h.rrf_score += rrf;
                if h.matched_symbol.is_none() {
                    h.matched_symbol = Some(sym.name.clone());
                    h.symbol_kind = Some(sym.kind);
                }
            })
            .or_insert(FusedHit {
                file_path: sym.file_path.clone(),
                line_start: sym.line as u32,
                content,
                rrf_score: rrf,
                vector_score: None,
                matched_symbol: Some(sym.name.clone()),
                symbol_kind: Some(sym.kind),
            });
    }

    // Content-body FTS (chunk text) — fills gaps symbols FTS misses.
    let content_fts = match ctx
        .sqlite
        .search_chunks_fts(&fts_query, (limit * 2) as i32, path_filter_abs.as_deref())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("chunks_fts search skipped: {}", e);
            Vec::new()
        }
    };
    for (rank, (file_path, line_start, content)) in content_fts.into_iter().enumerate() {
        let key = (file_path.clone(), line_start as u32);
        let rrf = 1.0 / (K + rank as f64 + 1.0) + path_boost(&file_path) + 0.05;
        scores
            .entry(key)
            .and_modify(|h| {
                h.rrf_score += rrf;
                if h.content.len() < content.len() {
                    h.content = content.clone();
                }
            })
            .or_insert(FusedHit {
                file_path,
                line_start: line_start as u32,
                content,
                rrf_score: rrf,
                vector_score: None,
                matched_symbol: None,
                symbol_kind: None,
            });
    }

    let fused: Vec<FusedHit> = scores.into_values().collect();
    let mut fused = fused;
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit * 3); // Keep extra for filtering / per-file cap

    // Glob filter: match full path and basename (not basename-only).
    let fused = if let Some(g) = glob_filter {
        let glob_pat = glob::Pattern::new(g).ok();
        let mut filtered: Vec<FusedHit> = fused
            .into_iter()
            .filter(|hit| {
                let Some(ref gp) = glob_pat else {
                    return true;
                };
                let rel = make_relative(&ctx.root_path, &hit.file_path);
                let name = Path::new(&hit.file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                gp.matches(&rel) || gp.matches(&hit.file_path) || gp.matches(name)
            })
            .collect();
        filtered.truncate(limit * 3);
        filtered
    } else {
        fused
    };

    // Score reporting:
    //
    // Previously the `[score=...]` field was the *normalized RRF rank score* —
    // 1.0 / (k + rank). The top result was always 1.000, regardless of whether
    // it was actually a semantic match. That's how mcp-registry.service.ts ended
    // up with `score=1.000` on a query about a feature that lived in a totally
    // different file.
    //
    // Now: when a vector-search match is available we surface the real cosine
    // similarity. When only FTS contributed we fall back to the RRF rank-score
    // and label it accordingly. Filtering by `min_score` uses the cosine when
    // present so callers can set a meaningful semantic threshold.
    let max_rrf = fused.first().map(|h| h.rrf_score).unwrap_or(1.0);
    let enhanced_results: Vec<(f32, Value)> = fused
        .into_iter()
        .map(|hit| {
            let rank_score = if max_rrf > 0.0 {
                (hit.rrf_score / max_rrf) as f32
            } else {
                0.0
            };
            // Headline score: real cosine similarity when we have it.
            let headline_score = hit.vector_score.unwrap_or(rank_score);

            let match_reason = determine_match_reason(&hit, query);

            let preview = if preview_mode {
                generate_preview(&hit.content, 3)
            } else {
                None
            };

            let context = if include_context {
                hit.matched_symbol.clone().or_else(|| {
                    extract_context_from_content(&hit.content)
                })
            } else {
                None
            };

            let default_content = hit.content.trim().to_string();
            let mut content_str = if preview_mode {
                preview.as_ref().unwrap_or(&default_content).clone()
            } else {
                default_content
            };
            let mut content_truncated = false;
            if content_str.len() > SEARCH_CONTENT_MAX_CHARS {
                // Prefer char boundary for UTF-8 safety.
                let mut end = SEARCH_CONTENT_MAX_CHARS;
                while end > 0 && !content_str.is_char_boundary(end) {
                    end -= 1;
                }
                content_str.truncate(end);
                content_str.push('…');
                content_truncated = true;
            }

            let mut result = json!({
                "file": make_relative(&ctx.root_path, &hit.file_path),
                "line": hit.line_start,
                "content": content_str,
            });
            if content_truncated {
                result["content_truncated"] = json!(true);
            }
            if include_scores {
                result["score"] = json!(headline_score);
                result["rank_score"] = json!(rank_score);
                if hit.vector_score.is_none() {
                    result["score_source"] = json!("fts");
                } else {
                    result["score_source"] = json!("vector");
                }
            }
            if let Some(reason) = match_reason {
                result["reason"] = json!(reason);
            }
            if let Some(ctx_val) = context {
                result["context"] = json!(ctx_val);
            }

            (headline_score, result)
        })
        .filter(|(score, _)| *score >= min_score)
        .collect::<Vec<_>>();

    // Sort by score descending, then diversify: max N hits per file.
    let mut enhanced_results = enhanced_results;
    enhanced_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut per_file: HashMap<String, usize> = HashMap::new();
    let mut diversified: Vec<Value> = Vec::with_capacity(limit);
    for (_, r) in enhanced_results {
        let file = r
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let n = per_file.entry(file).or_insert(0);
        if *n >= max_per_file {
            continue;
        }
        *n += 1;
        diversified.push(r);
        if diversified.len() >= limit {
            break;
        }
    }

    let results = diversified;
    let search_time_ms = search_start.elapsed().as_millis();

    // Index coverage check — surfaces "embeddings 71% complete" so the caller
    // knows results may be missing recent files. file_count comes from sqlite,
    // chunk_count from LanceDB (the actual vector store).
    let file_count = ctx.sqlite.get_file_count().await.unwrap_or(0);
    let chunk_count = ctx.lance.count().await.unwrap_or(0) as i64;
    if file_count > 10 {
        let ratio = chunk_count as f64 / file_count as f64;
        // bge-m3 typically produces 3+ chunks/file in well-indexed projects.
        // Below 1.0 chunks/file the index is meaningfully incomplete and recent
        // edits are likely unsearchable.
        if ratio < 1.0 {
            warnings.push(format!(
                "Index coverage degraded: {:.2} chunks/file ({} files, {} chunks). Recent commits may be missing — call reindex force=true.",
                ratio, file_count, chunk_count
            ));
            degraded = true;
        }
    }

    // 6. Structured output with degraded mode info (Feature 016)
    let mut final_result = json!({
        "query": query,
        "total_results": results.len(),
        "results": results,
        "search_time_ms": search_time_ms
    });

    // Add degraded mode information if applicable
    if degraded {
        if let Some(obj) = final_result.as_object_mut() {
            obj.insert("degraded".to_string(), json!(true));
            obj.insert("warnings".to_string(), json!(warnings));
        }
    }

    // Store in cache (only if not degraded for best quality)
    if !degraded {
        if let Ok(result_json) = serde_json::to_string(&final_result) {
            ctx.cache
                .put_search(cache_fingerprint, 0, result_json)
                .await;
        }
    }

    Ok(final_result)
}


// === Helper Functions ===

fn determine_match_reason(hit: &FusedHit, query: &str) -> Option<String> {
    let query_lower = query.to_lowercase();
    let content_lower = hit.content.to_lowercase();

    // Check if matched via symbol name
    if let Some(ref symbol) = hit.matched_symbol {
        let sym_l = symbol.to_lowercase();
        let exact = query
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|t| t.len() >= 2 && t.eq_ignore_ascii_case(symbol));
        if exact || sym_l.contains(&query_lower) || query_lower.contains(&sym_l) {
            let kind = match hit.symbol_kind.as_ref() {
                Some(SymbolKind::Function) => "FunctionName",
                Some(SymbolKind::Struct) | Some(SymbolKind::Class) => "ClassName",
                Some(SymbolKind::Enum) => "TypeDefinition",
                Some(SymbolKind::Trait) | Some(SymbolKind::Interface) => "TypeDefinition",
                _ => "SymbolName",
            };
            return Some(if exact {
                format!("{}Exact", kind)
            } else {
                kind.to_string()
            });
        }
    }

    // Check for doc comments
    if hit.content.contains("///") || hit.content.contains("/**") || hit.content.contains("\"\"\"")
    {
        let doc_start = hit
            .content
            .find("///")
            .or_else(|| hit.content.find("/**"))
            .or_else(|| hit.content.find("\"\"\""));

        if let Some(pos) = doc_start {
            // Safe UTF-8 substring: find char boundary instead of using byte offset
            let end_pos = hit.content[pos..]
                .char_indices()
                .take(200)
                .last()
                .map(|(i, c)| pos + i + c.len_utf8())
                .unwrap_or(pos);
            let doc_section = &hit.content[pos..end_pos.min(hit.content.len())];
            if doc_section.to_lowercase().contains(&query_lower) {
                return Some("DocComment".to_string());
            }
        }
    }

    // Check for import statements
    if (hit.content.contains("use ")
        || hit.content.contains("import ")
        || hit.content.contains("from "))
        && content_lower.contains(&query_lower)
    {
        return Some("ImportStatement".to_string());
    }

    // Check if it's a type definition line
    if (hit.content.contains("struct ")
        || hit.content.contains("enum ")
        || hit.content.contains("class ")
        || hit.content.contains("interface ")
        || hit.content.contains("type "))
        && content_lower.contains(&query_lower)
    {
        return Some("TypeDefinition".to_string());
    }

    // Default: matched in code content
    Some("CodeContent".to_string())
}

fn generate_preview(content: &str, max_lines: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().take(max_lines).collect();
    if lines.is_empty() {
        None
    } else {
        let preview = lines.join("\n");
        Some(if content.lines().count() > max_lines {
            format!("{}...", preview)
        } else {
            preview
        })
    }
}

fn extract_context_from_content(content: &str) -> Option<String> {
    // Try to find function definition
    for line in content.lines().take(5) {
        let trimmed = line.trim();

        // Rust: pub fn name / fn name
        if let Some(pos) = trimmed.find(" fn ") {
            if let Some(name_start) =
                trimmed[pos + 4..].find(|c: char| c.is_alphanumeric() || c == '_')
            {
                if let Some(name_end) = trimmed[pos + 4 + name_start..].find(['(', '<']) {
                    return Some(
                        trimmed[pos + 4 + name_start..pos + 4 + name_start + name_end].to_string(),
                    );
                }
            }
        }

        // TypeScript/JavaScript: function name / const name =
        if trimmed.starts_with("function ") || trimmed.starts_with("export function ") {
            if let Some(name) = trimmed.split_whitespace().nth(1) {
                let name = name.trim_end_matches('(');
                return Some(name.to_string());
            }
        }

        if trimmed.contains("const ") && trimmed.contains(" = ") {
            if let Some(start) = trimmed.find("const ") {
                if let Some(end) = trimmed[start + 6..].find(" = ") {
                    return Some(trimmed[start + 6..start + 6 + end].trim().to_string());
                }
            }
        }

        // Python: def name
        if let Some(stripped) = trimmed.strip_prefix("def ") {
            if let Some(name) = stripped.split('(').next() {
                return Some(name.trim().to_string());
            }
        }

        // Classes
        if trimmed.contains("class ") {
            if let Some(start) = trimmed.find("class ") {
                let after_class = &trimmed[start + 6..];
                if let Some(name_end) = after_class.find([' ', '{', '(', '<']) {
                    return Some(after_class[..name_end].trim().to_string());
                }
            }
        }

        // Structs/Enums
        if trimmed.contains("struct ") || trimmed.contains("enum ") {
            let keyword = if trimmed.contains("struct ") {
                "struct "
            } else {
                "enum "
            };
            if let Some(start) = trimmed.find(keyword) {
                let after_keyword = &trimmed[start + keyword.len()..];
                if let Some(name_end) = after_keyword.find([' ', '{', '<']) {
                    return Some(after_keyword[..name_end].trim().to_string());
                }
            }
        }
    }

    None
}

