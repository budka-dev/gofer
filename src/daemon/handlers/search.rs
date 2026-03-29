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

    if query.is_empty() {
        return Err(GoferError::InvalidParams("Query is required".into()).into());
    }

    // NEW: Feature 008 - Check cache first
    if let Some(cached_json) = ctx.cache.get_search(query, limit).await {
        // Parse cached JSON back to Value
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

    // Vector results contribute
    for (rank, hit) in vector_results.iter().enumerate() {
        let key = (hit.file_path.clone(), hit.line_start);
        let rrf = 1.0 / (K + rank as f64 + 1.0);
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

    // FTS results contribute
    for (rank, sym) in fts_results.iter().enumerate() {
        let key = (sym.file_path.clone(), sym.line as u32);
        let rrf = 1.0 / (K + rank as f64 + 1.0);
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

    let fused: Vec<FusedHit> = scores.into_values().collect();
    let mut fused = fused;
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit * 2); // Keep extra for filtering

    // 5. Filter by path/glob if specified - NOW ONLY FOR GLOB
    let glob_filter = args.get("glob").and_then(|v| v.as_str());

    let fused = if glob_filter.is_some() {
        let glob_pat = glob_filter.and_then(|g| glob::Pattern::new(g).ok());

        let mut filtered: Vec<FusedHit> = fused
            .into_iter()
            .filter(|hit| {
                if let Some(ref gp) = glob_pat {
                    let name = Path::new(&hit.file_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if !gp.matches(name) {
                        return false;
                    }
                }
                true
            })
            .collect();
        filtered.truncate(limit * 2);
        filtered
    } else {
        fused
    };

    // NEW: Normalize scores and filter by min_score
    let max_rrf = fused.first().map(|h| h.rrf_score).unwrap_or(1.0);
    let enhanced_results: Vec<(f32, Value)> = fused
        .into_iter()
        .map(|hit| {
            // Normalize RRF score to 0.0-1.0
            let normalized_score = if max_rrf > 0.0 {
                (hit.rrf_score / max_rrf) as f32
            } else {
                0.0
            };

            // Determine match reason
            let match_reason = determine_match_reason(&hit, query);

            // Generate preview if requested
            let preview = if preview_mode {
                generate_preview(&hit.content, 3)
            } else {
                None
            };

            // Get context (symbol name)
            let context = if include_context {
                hit.matched_symbol.clone().or_else(|| {
                    // Try to extract function/class name from content
                    extract_context_from_content(&hit.content)
                })
            } else {
                None
            };

            let default_content = hit.content.trim().to_string();
            let content_str = if preview_mode {
                preview.as_ref().unwrap_or(&default_content)
            } else {
                &default_content
            };

            let mut parts = vec![format!(
                "{}:{}",
                make_relative(&ctx.root_path, &hit.file_path),
                hit.line_start
            )];

            if include_scores {
                parts.push(format!("[score={:.3}]", normalized_score));
            }
            if include_scores || preview_mode {
                if let Some(reason) = &match_reason {
                    parts.push(format!("(reason:{})", reason));
                }
            }
            if let Some(ctx_val) = context {
                parts.push(format!("(ctx:{})", ctx_val));
            }

            let result = json!(format!("{}\n{}", parts.join(" "), content_str));

            (normalized_score, result)
        })
        .filter(|(score, _)| *score >= min_score)
        .collect::<Vec<_>>();

    // Sort by score descending
    let mut enhanced_results = enhanced_results;
    enhanced_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    enhanced_results.truncate(limit);

    let results: Vec<Value> = enhanced_results.into_iter().map(|(_, r)| r).collect();
    let search_time_ms = search_start.elapsed().as_millis();

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

    // NEW: Feature 008 - Store in cache (only if not degraded for best quality)
    if !degraded {
        if let Ok(result_json) = serde_json::to_string(&final_result) {
            ctx.cache
                .put_search(query.to_string(), limit, result_json)
                .await;
        }
    }

    Ok(final_result)
}

#[allow(dead_code)]
pub async fn tool_cross_stack_search(args: Value, ctx: &ToolContext) -> Result<Value> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let include_links = args
        .get("include_links")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if query.is_empty() {
        return Err(GoferError::InvalidParams("Query is required".into()).into());
    }

    // Run normal hybrid search first
    let search_result = tool_search(args, ctx).await?;

    if !include_links {
        return Ok(search_result);
    }

    // Collect unique file paths from the structured search results
    let result_files: Vec<String> = search_result
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    r.get("file_path")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();

    if result_files.is_empty() {
        return Ok(search_result);
    }

    // Look up cross-stack links for each file
    let mut seen_links = std::collections::HashSet::new();
    let mut links = Vec::new();

    for file_path in &result_files {
        let abs_path = resolve_path(&ctx.root_path, file_path);
        let file_links = match ctx.sqlite.get_cross_stack_links_for_file(&abs_path).await {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!("Failed to get cross-stack links for {}: {}", file_path, e);
                Vec::new()
            }
        };

        for link in &file_links {
            let key = format!("{}->{}", link.source_symbol, link.target_symbol);
            if seen_links.insert(key) {
                links.push(json!(format!(
                    "{}:{} -> {}:{} ({}) [w={:.2}]",
                    make_relative(&ctx.root_path, &link.source_file),
                    link.source_symbol,
                    make_relative(&ctx.root_path, &link.target_file),
                    link.target_symbol,
                    link.link_type,
                    link.weight
                )));
            }
        }
    }

    // Merge links into the search result
    let mut result = search_result;
    result["cross_stack_links"] = json!(links);
    Ok(result)
}

pub async fn tool_search_by_purpose(args: Value, ctx: &ToolContext) -> Result<Value> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    if query.is_empty() {
        return Err(GoferError::InvalidParams("Query is required".into()).into());
    }

    // Strategy: combine vector search (semantic) with keyword matching on summaries.
    // 1. Vector search over code chunks, group by file
    let embedding = ctx.embedder.embed_query(query).await?;

    let vector_hits = {
        match ctx.lance.search(&embedding, limit * 3).await {
            Ok(hits) => hits,
            Err(e) => {
                tracing::error!("Vector search failed: {}", e);
                Vec::new()
            }
        }
    };

    // Deduplicate by file, keep best score per file
    let mut file_scores: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    for hit in &vector_hits {
        file_scores
            .entry(hit.file_path.clone())
            .and_modify(|s| {
                if hit.score > *s {
                    *s = hit.score;
                }
            })
            .or_insert(hit.score);
    }

    // 3. Rank and format
    let mut ranked: Vec<(String, f32)> = file_scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(limit);

    Ok(json!({
        "query": query,
        "total": ranked.len(),
        "files": ranked.iter().map(|(file_path, score)| {
            format!("{} [score={:.4}]", make_relative(&ctx.root_path, file_path), score)
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_smart_file_selection(args: Value, ctx: &ToolContext) -> Result<Value> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let min_score = args
        .get("min_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.3) as f32;

    if query.is_empty() {
        return Err(GoferError::InvalidParams("Query is required".into()).into());
    }

    // Check cache first (using get_search as a substitute for get_file_selection)
    if let Some(cached_json) = ctx.cache.get_search(query, limit).await {
        if let Ok(cached_result) = serde_json::from_str::<Value>(&cached_json) {
            return Ok(cached_result);
        }
    }

    // 1. Vector search for semantic similarity
    let embedding = ctx.embedder.embed_query(query).await?;
    let vector_results = { ctx.lance.search(&embedding, limit * 3).await? };

    // 2. Extract unique files from vector results
    use std::collections::HashMap;
    let mut file_scores: HashMap<String, f32> = HashMap::new();
    let mut file_chunks: HashMap<String, Vec<String>> = HashMap::new();

    for hit in &vector_results {
        let score = *file_scores.get(&hit.file_path).unwrap_or(&0.0);
        file_scores.insert(hit.file_path.clone(), score.max(hit.score));

        file_chunks
            .entry(hit.file_path.clone())
            .or_default()
            .push(hit.content.clone());
    }

    // 3. Get file metadata and symbols
    let mut candidates = Vec::new();
    let total_candidates = file_scores.len();

    for (file_path, vector_score) in file_scores {
        // Get symbols in this file
        let symbols: Vec<crate::models::chunk::SymbolWithPath> = ctx
            .sqlite
            .get_symbols(Some(&file_path), None, 0, 500)
            .await
            .unwrap_or_default();

        // Get file metadata for scoring
        let file_metadata = get_file_metadata(&file_path).await;

        // Calculate component scores with v2 algorithm
        let path_score = calculate_path_score_v2(query, &file_path);
        let symbol_score = calculate_symbol_score(query, &symbols);

        // Calculate score with adaptive weights, recency, and size
        let (final_score, scoring_details) = calculate_relevance_score_v2(
            query,
            &file_metadata,
            vector_score,
            path_score,
            symbol_score,
        );

        // Generate reasoning
        let reason = generate_selection_reason_v2(&scoring_details, &symbols, query);

        // Extract key symbols
        let key_symbols: Vec<String> = symbols.iter().map(|s| s.name.clone()).take(5).collect();

        candidates.push((
            final_score,
            json!(format!(
                "{} [score={:.3}]\nReason: {}\nSymbols: {}",
                make_relative(&ctx.root_path, &file_path),
                final_score,
                reason,
                key_symbols.join(", ")
            )),
        ));
    }

    // Sort by score descending
    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Filter by min_score and limit
    let ranked_files: Vec<Value> = candidates
        .into_iter()
        .filter(|(score, _)| *score >= min_score)
        .take(limit)
        .map(|(_, file)| file)
        .collect();

    // Generate overall reasoning
    let reasoning = if ranked_files.is_empty() {
        format!(
            "No files found matching query '{}' with score >= {}",
            query, min_score
        )
    } else {
        format!("Found {} relevant files for '{}' using vector search, path analysis, and symbol matching",
            ranked_files.len(), query)
    };

    let result = json!({
        "files": ranked_files,
        "reasoning": reasoning,
        "total_candidates": total_candidates
    });

    // Store in cache (using put_search as a substitute for put_file_selection)
    if let Ok(result_json) = serde_json::to_string(&result) {
        ctx.cache
            .put_search(query.to_string(), limit, result_json)
            .await;
    }

    Ok(result)
}

// === Helper Functions ===

fn determine_match_reason(hit: &FusedHit, query: &str) -> Option<String> {
    let query_lower = query.to_lowercase();
    let content_lower = hit.content.to_lowercase();

    // Check if matched via symbol name
    if let Some(ref symbol) = hit.matched_symbol {
        if symbol.to_lowercase().contains(&query_lower) {
            // Check symbol kind to be more specific
            return Some(
                match hit.symbol_kind.as_ref() {
                    Some(SymbolKind::Function) => "FunctionName",
                    Some(SymbolKind::Struct) | Some(SymbolKind::Class) => "ClassName",
                    Some(SymbolKind::Enum) => "TypeDefinition",
                    Some(SymbolKind::Trait) | Some(SymbolKind::Interface) => "TypeDefinition",
                    _ => "SymbolName",
                }
                .to_string(),
            );
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

/// File metadata for scoring
struct FileMetadata {
    size_bytes: usize,
    last_modified: std::time::SystemTime,
}

/// Get file metadata
async fn get_file_metadata(path: &str) -> FileMetadata {
    let file_path = std::path::Path::new(path);
    if let Ok(metadata) = tokio::fs::metadata(file_path).await {
        FileMetadata {
            size_bytes: metadata.len() as usize,
            last_modified: metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        }
    } else {
        FileMetadata {
            size_bytes: 0,
            last_modified: std::time::SystemTime::UNIX_EPOCH,
        }
    }
}

/// Scoring weights for different query types
#[derive(Debug)]
struct ScoringWeights {
    vector: f32,
    path: f32,
    symbols: f32,
}

/// Scoring details for transparency
#[derive(Debug)]
#[allow(dead_code)]
struct ScoringDetails {
    base_score: f32,
    recency_boost: f32,
    size_penalty: f32,
    confidence: f32,
    weights: ScoringWeights,
}

/// Calculate relevance score with adaptive weights v2
fn calculate_relevance_score_v2(
    query: &str,
    file_metadata: &FileMetadata,
    vector_score: f32,
    path_score: f32,
    symbol_score: f32,
) -> (f32, ScoringDetails) {
    // 1. Determine query type and adjust weights
    let weights = calculate_adaptive_weights(query);

    // 2. Calculate recency boost
    let recency_boost = calculate_recency_boost(file_metadata.last_modified);

    // 3. Calculate size penalty
    let size_penalty = calculate_size_penalty(file_metadata.size_bytes);

    // 4. Calculate base score
    let base_score =
        vector_score * weights.vector + path_score * weights.path + symbol_score * weights.symbols;

    // 5. Apply modifiers
    let final_score = base_score * recency_boost * size_penalty;

    // 6. Calculate confidence
    let confidence = calculate_confidence(vector_score, path_score, symbol_score);

    let details = ScoringDetails {
        base_score,
        recency_boost,
        size_penalty,
        confidence,
        weights,
    };

    (final_score.clamp(0.0, 1.0), details)
}

/// Adaptive weights based on query analysis
fn calculate_adaptive_weights(query: &str) -> ScoringWeights {
    let query_lower = query.to_lowercase();

    // Pattern 1: "where is X defined?" -> prioritize symbols
    if query_lower.contains("where")
        || query_lower.contains("defined")
        || query_lower.contains("find")
    {
        return ScoringWeights {
            vector: 0.25,
            path: 0.15,
            symbols: 0.60, // Boost symbols
        };
    }

    // Pattern 2: "how does X work?" -> prioritize path and symbols
    if query_lower.contains("how")
        || query_lower.contains("explain")
        || query_lower.contains("what")
    {
        return ScoringWeights {
            vector: 0.40,
            path: 0.30,
            symbols: 0.30,
        };
    }

    // Pattern 3: file path mentioned -> prioritize path
    if query.contains("/")
        || query.contains(".rs")
        || query.contains(".ts")
        || query.contains(".js")
    {
        return ScoringWeights {
            vector: 0.40,
            path: 0.40, // Boost path
            symbols: 0.20,
        };
    }

    // Default: balanced
    ScoringWeights {
        vector: 0.50,
        path: 0.25,
        symbols: 0.25,
    }
}

/// Recency boost: recently modified files are more relevant
fn calculate_recency_boost(last_modified: std::time::SystemTime) -> f32 {
    let age = std::time::SystemTime::now()
        .duration_since(last_modified)
        .unwrap_or_default();

    let days_old = age.as_secs() / 86400;

    match days_old {
        0..=1 => 1.15,   // Modified today/yesterday: +15%
        2..=7 => 1.05,   // This week: +5%
        8..=30 => 1.0,   // This month: no change
        31..=90 => 0.95, // Last 3 months: -5%
        _ => 0.90,       // Older: -10%
    }
}

/// Size penalty: very large files are harder to work with
fn calculate_size_penalty(size_bytes: usize) -> f32 {
    let size_kb = size_bytes / 1024;

    match size_kb {
        0..=50 => 1.0,      // < 50KB: no penalty
        51..=200 => 0.98,   // 50-200KB: tiny penalty
        201..=500 => 0.95,  // 200-500KB: small penalty
        501..=1000 => 0.90, // 500KB-1MB: medium penalty
        _ => 0.85,          // > 1MB: large penalty
    }
}

/// Confidence: how confident are we in this ranking?
fn calculate_confidence(vector: f32, path: f32, symbol: f32) -> f32 {
    // High confidence if multiple signals agree
    let signals = [vector, path, symbol];
    let mean = signals.iter().sum::<f32>() / signals.len() as f32;
    let variance = signals.iter().map(|&s| (s - mean).powi(2)).sum::<f32>() / signals.len() as f32;

    // Low variance = high confidence
    let confidence = 1.0 - variance.sqrt();
    confidence.clamp(0.0, 1.0)
}

/// Generate reasoning for file selection v2
#[allow(dead_code)]
fn generate_selection_reason_v2(
    details: &ScoringDetails,
    symbols: &[crate::models::chunk::SymbolWithPath],
    _query: &str,
) -> String {
    let mut reasons: Vec<String> = Vec::new();

    // Semantic similarity
    let vector_component = details.base_score * details.weights.vector;
    if vector_component > 0.28 {
        reasons.push("high semantic similarity".to_string());
    } else if vector_component > 0.20 {
        reasons.push("good semantic match".to_string());
    }

    // Path matching
    let path_component = details.base_score * details.weights.path;
    if path_component > 0.12 {
        reasons.push("path matches query".to_string());
    }

    // Symbol matching
    let symbol_component = details.base_score * details.weights.symbols;
    if symbol_component > 0.15 && !symbols.is_empty() {
        let symbol_names = symbols
            .iter()
            .take(3)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        reasons.push(format!("contains relevant symbols ({})", symbol_names));
    }

    // Recency
    if details.recency_boost > 1.0 {
        reasons.push("recently modified".to_string());
    }

    // Confidence
    if details.confidence > 0.8 {
        reasons.push("high confidence".to_string());
    }

    if reasons.is_empty() {
        "found in search results".to_string()
    } else {
        reasons.join(", ")
    }
}

/// Calculate path relevance score v2 (improved with edit distance)
#[allow(dead_code)]
fn calculate_path_score_v2(query: &str, path: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let path_lower = path.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();

    let mut score: f32 = 0.0;

    // Extract path components
    let filename = path_lower.split('/').next_back().unwrap_or("");
    let filename_stem = filename.split('.').next().unwrap_or("");
    let directories: Vec<&str> = path_lower.split('/').collect();

    for keyword in &keywords {
        // Normalize keyword (basic stemming)
        let normalized_keyword = normalize_keyword(keyword);

        // 1. Exact filename match (highest priority)
        if filename_stem == normalized_keyword {
            score += 0.5;
            continue;
        }

        // 2. Filename contains keyword
        if filename_stem.contains(&normalized_keyword) {
            let similarity = similarity_ratio(&normalized_keyword, filename_stem);
            score += 0.3 * similarity;
            continue;
        }

        // 3. Directory matches
        let important_dirs = ["src", "lib", "core", "api", "components", "services"];
        for (idx, dir) in directories.iter().enumerate() {
            if dir.contains(&normalized_keyword) {
                let importance = if important_dirs.contains(dir) {
                    1.2
                } else {
                    1.0
                };
                let proximity = 1.0 / (directories.len() - idx).max(1) as f32;
                score += 0.15 * importance * proximity;
            }
        }

        // 4. Fuzzy match using edit distance
        if edit_distance(keyword, filename_stem) <= 2 {
            score += 0.2;
        }
    }

    score.min(1.0)
}

/// Normalize keyword (basic stemming)
fn normalize_keyword(word: &str) -> String {
    let mut word = word.to_string();
    // Remove common suffixes
    if word.ends_with("ing") && word.len() > 5 {
        word.truncate(word.len() - 3);
    } else if word.ends_with("ed") && word.len() > 4 {
        word.truncate(word.len() - 2);
    } else if word.ends_with("s") && word.len() > 3 {
        word.truncate(word.len() - 1);
    }
    word
}

/// Calculate similarity ratio
fn similarity_ratio(a: &str, b: &str) -> f32 {
    let matches = a.chars().filter(|c| b.contains(*c)).count();

    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }

    matches as f32 / max_len as f32
}

/// Levenshtein edit distance
fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

    for (i, row) in matrix.iter_mut().enumerate().take(a_len + 1) {
        row[0] = i;
    }
    for (j, item) in matrix[0].iter_mut().enumerate().take(b_len + 1) {
        *item = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = *[
                matrix[i - 1][j] + 1,        // deletion
                matrix[i][j - 1] + 1,        // insertion
                matrix[i - 1][j - 1] + cost, // substitution
            ]
            .iter()
            .min()
            .unwrap();
        }
    }

    matrix[a_len][b_len]
}

/// Calculate path relevance score (original - kept for compatibility)
#[allow(dead_code)]
fn calculate_path_score(query: &str, path: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let path_lower = path.to_lowercase();

    let mut score: f32 = 0.0;

    // Split query into keywords
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();

    for keyword in keywords {
        // Check path components
        if path_lower.contains(keyword) {
            // Higher score for filename match vs directory
            if path_lower
                .split('/')
                .next_back()
                .unwrap_or("")
                .contains(keyword)
            {
                score += 0.3;
            } else {
                score += 0.1;
            }
        }
    }

    score.min(1.0)
}

/// Calculate symbol relevance score
fn calculate_symbol_score(query: &str, symbols: &[crate::models::chunk::SymbolWithPath]) -> f32 {
    if symbols.is_empty() {
        return 0.0;
    }

    let query_lower = query.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();

    let mut matches = 0;
    for symbol in symbols {
        let symbol_lower = symbol.name.to_lowercase();
        for keyword in &keywords {
            if symbol_lower.contains(keyword) {
                matches += 1;
                break;
            }
        }
    }

    (matches as f32 / keywords.len() as f32).min(1.0)
}

/// Generate reasoning for file selection
#[allow(dead_code)]
fn generate_selection_reason(
    vector_score: f32,
    path_score: f32,
    symbol_score: f32,
    symbols: &[crate::models::chunk::SymbolWithPath],
    _query: &str,
) -> String {
    let mut reasons: Vec<String> = Vec::new();

    if vector_score > 0.7 {
        reasons.push("high semantic similarity".to_string());
    } else if vector_score > 0.5 {
        reasons.push("good semantic match".to_string());
    }

    if path_score > 0.3 {
        reasons.push("path matches query keywords".to_string());
    }

    if symbol_score > 0.5 && !symbols.is_empty() {
        let symbol_names = symbols
            .iter()
            .take(3)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        reasons.push(format!("contains relevant symbols ({})", symbol_names));
    }

    if reasons.is_empty() {
        "found in search results".to_string()
    } else {
        reasons.join(", ")
    }
}
