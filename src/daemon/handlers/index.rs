use super::common::{resolve_path, ToolContext};
use crate::error::GoferError;
use crate::indexer::service::IndexerService;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub async fn tool_get_index_status(ctx: &ToolContext) -> Result<Value> {
    use std::time::Instant;
    let start = Instant::now();

    // Get basic counts
    let file_count = ctx.sqlite.get_file_count().await?;
    let symbol_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM symbols")
        .fetch_one(ctx.sqlite.pool())
        .await?;

    // Get chunk count from LanceDB
    let chunk_count = { ctx.lance.count().await.unwrap_or(0) };

    // Get index metadata
    #[derive(sqlx::FromRow)]
    struct MetadataRow {
        key: String,
        value: String,
    }

    let metadata: Vec<MetadataRow> = sqlx::query_as(
        r#"
        SELECT key, value
        FROM index_metadata
        WHERE key IN ('last_full_sync', 'indexing_started_at', 'indexing_completed_at')
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    let mut meta_map = std::collections::HashMap::new();
    for row in metadata {
        meta_map.insert(row.key, row.value);
    }

    // Get pending/failed/completed files
    #[derive(sqlx::FromRow)]
    struct StatusCount {
        indexing_status: Option<String>,
        count: i64,
    }

    let status_counts: Vec<StatusCount> = sqlx::query_as(
        r#"
        SELECT indexing_status, COUNT(*) as count
        FROM files
        GROUP BY indexing_status
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    let mut pending = 0i64;
    let mut failed = 0i64;
    let mut completed = 0i64;

    for row in status_counts {
        let count = row.count;
        match row.indexing_status.as_deref() {
            Some("pending") => pending = count,
            Some("failed") => failed = count,
            Some("completed") => completed += count,
            None => completed += count, // NULL means successfully indexed (old behavior)
            _ => {}
        }
    }

    // Calculate completeness percentage
    let total_files = file_count as f64;
    let completeness = if total_files > 0.0 {
        (completed as f64 / total_files * 100.0).min(100.0)
    } else {
        0.0
    };

    // Calculate age since last sync
    let empty_string = String::new();
    let last_sync_str = meta_map.get("last_full_sync").unwrap_or(&empty_string);
    let age_minutes = if !last_sync_str.is_empty() {
        if let Ok(last_sync) = chrono::DateTime::parse_from_rfc3339(last_sync_str) {
            let now = chrono::Utc::now();
            (now.timestamp() - last_sync.timestamp()) / 60
        } else {
            -1
        }
    } else {
        -1
    };

    // Determine IndexHealth based on spec criteria
    let health = if completeness > 95.0 && pending == 0 && (0..60).contains(&age_minutes) {
        "Healthy"
    } else if completeness > 80.0 && pending < 10 && age_minutes < 1440 {
        "Degraded"
    } else {
        "Unhealthy"
    };

    // Get symbol breakdown by kind
    #[derive(sqlx::FromRow)]
    struct SymbolKindCount {
        kind: String,
        count: i64,
    }

    let symbol_breakdown: Vec<SymbolKindCount> = sqlx::query_as(
        r#"
        SELECT kind, COUNT(*) as count
        FROM symbols
        GROUP BY kind
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    let mut symbols_by_kind = serde_json::Map::new();
    for row in symbol_breakdown {
        symbols_by_kind.insert(
            row.kind.clone(),
            serde_json::Value::Number(row.count.into()),
        );
    }

    // Generate warnings
    let mut warnings = Vec::new();
    let mut recommendations = Vec::new();

    if failed > 0 {
        warnings.push(format!("[error] {} files failed to index", failed));
        recommendations.push("Call reindex path=<file> for failed files, or reindex force=true".to_string());
    }

    if pending > 0 {
        warnings.push(format!("[info] {} files pending indexing", pending));
        recommendations.push("Wait for indexing to complete or check daemon logs".to_string());
    }

    if age_minutes > 1440 {
        warnings.push(format!(
            "[warning] Index not synced in {} hours",
            age_minutes / 60
        ));
        recommendations.push("Run reindex force=true to refresh index".to_string());
    }

    if completeness < 80.0 {
        warnings.push(format!(
            "[warning] Index only {:.1}% complete",
            completeness
        ));
        recommendations.push("Check for indexing errors and run validate_index".to_string());
    }

    // Check embedding ratio
    let embedding_ratio = if file_count > 0 {
        chunk_count as f64 / file_count as f64
    } else {
        0.0
    };

    if file_count > 10 && embedding_ratio < 1.0 {
        warnings.push(format!(
            "[warning] Low embedding ratio: {:.2} chunks per file",
            embedding_ratio
        ));
        recommendations
            .push("Some files may lack embeddings. Run validate_index for details".to_string());
    }

    // Reference graph quality
    let (ref_total, ref_unresolved): (i64, i64) = {
        let row = sqlx::query_as::<_, (i64, i64)>(
            r#"
            SELECT COUNT(*),
                   COALESCE(SUM(CASE WHEN target_symbol_id IS NULL THEN 1 ELSE 0 END), 0)
            FROM symbol_references
            "#,
        )
        .fetch_one(ctx.sqlite.pool())
        .await
        .unwrap_or((0, 0));
        row
    };
    let ref_resolved_pct = if ref_total > 0 {
        ((ref_total - ref_unresolved) as f64 / ref_total as f64) * 100.0
    } else {
        100.0
    };
    if ref_total > 100 && ref_resolved_pct < 50.0 {
        warnings.push(format!(
            "[warning] Only {:.1}% of references resolved to symbol ids ({} / {})",
            ref_resolved_pct,
            ref_total - ref_unresolved,
            ref_total
        ));
        recommendations.push(
            "Call reindex force=true or reindex path= on hot files so resolve_references can bind edges"
                .into(),
        );
    }

    // Embedder probe (short timeout — status must stay snappy)
    let embedder_status = match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        ctx.embedder.health_check(),
    )
    .await
    {
        Ok(Ok(())) => json!({
            "ok": true,
            "model": ctx.embedder.model_name(),
            "dimension": ctx.embedder.dimension(),
        }),
        Ok(Err(e)) => {
            warnings.push(format!("[error] Embedder unhealthy: {}", e));
            recommendations.push(
                "Start the embed HTTP service (default http://127.0.0.1:8080/embed/). Symbol tools still work; search degrades to FTS.".into(),
            );
            json!({
                "ok": false,
                "error": e.to_string(),
                "model": ctx.embedder.model_name(),
                "dimension": ctx.embedder.dimension(),
            })
        }
        Err(_) => {
            warnings.push("[error] Embedder health check timed out (3s)".into());
            recommendations.push(
                "Embedder is slow or unreachable. Search will degrade until it responds.".into(),
            );
            json!({
                "ok": false,
                "error": "timeout",
                "model": ctx.embedder.model_name(),
                "dimension": ctx.embedder.dimension(),
            })
        }
    };

    if warnings.is_empty() {
        recommendations.push("Index is healthy and up to date".to_string());
    }

    let elapsed_ms = start.elapsed().as_millis();

    Ok(json!({
        "health": health,
        "embedder": embedder_status,
        "references": {
            "total": ref_total,
            "unresolved": ref_unresolved,
            "resolved_percent": format!("{:.1}", ref_resolved_pct),
        },
        "status": if pending == 0 && failed == 0 { "complete" } else if pending > 0 { "indexing" } else { "partial" },
        "completeness": {
            "overall_percent": format!("{:.1}", completeness),
            "files_percent": format!("{:.1}", (completed as f64 / total_files * 100.0).min(100.0)),
            "symbols_percent": if file_count > 0 {
                format!("{:.1}", (symbol_count as f64 / file_count as f64 * 100.0).min(100.0))
            } else {
                "0.0".to_string()
            },
            "embeddings_percent": if file_count > 0 {
                format!("{:.1}", (embedding_ratio * 100.0).min(100.0))
            } else {
                "0.0".to_string()
            }
        },
        "files": {
            "total": file_count,
            "completed": completed,
            "pending": pending,
            "failed": failed,
            "oldest_indexed": meta_map.get("indexing_started_at").unwrap_or(&String::new())
        },
        "symbols": {
            "total": symbol_count,
            "by_kind": symbols_by_kind
        },
        "embeddings": {
            "total_chunks": chunk_count,
            "avg_chunks_per_file": format!("{:.2}", embedding_ratio)
        },
        "last_sync": last_sync_str,
        "age_minutes": age_minutes,
        "indexing_started_at": meta_map.get("indexing_started_at").unwrap_or(&String::new()),
        "indexing_completed_at": meta_map.get("indexing_completed_at").unwrap_or(&String::new()),
        "warnings": warnings,
        "recommendations": recommendations,
        "query_time_ms": elapsed_ms
    }))
}

pub async fn tool_validate_index(ctx: &ToolContext) -> Result<Value> {
    use std::time::Instant;
    let start = Instant::now();

    let mut issues = Vec::new();

    // Validator 1: Files without symbols (for languages that should have symbols)
    #[derive(sqlx::FromRow)]
    struct FileWithoutSymbols {
        path: String,
        language: Option<String>,
    }

    let files_without_symbols: Vec<FileWithoutSymbols> = sqlx::query_as(
        r#"
        SELECT f.path, f.language
        FROM files f
        LEFT JOIN symbols s ON s.file_id = f.id
        WHERE s.id IS NULL
        AND f.language IN ('rust', 'typescript', 'python', 'go', 'javascript')
        LIMIT 20
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    if !files_without_symbols.is_empty() {
        let sample_files: Vec<String> = files_without_symbols
            .iter()
            .take(10)
            .map(|r| {
                format!(
                    "{} ({})",
                    r.path,
                    r.language.as_deref().unwrap_or("unknown")
                )
            })
            .collect();

        issues.push(json!({
            "id": "missing_symbols_001",
            "severity": "high",
            "category": "missing_data",
            "message": "Files indexed without symbols extracted",
            "details": {
                "description": format!("{} code files have no symbols in index", files_without_symbols.len()),
                "impact": "These files won't appear in symbol search results",
                "root_cause": "Files may be empty, parsing failed, or file is not valid code",
                "examples": sample_files
            },
            "affected_items": files_without_symbols.iter().map(|r| format!("file: {}", r.path)).collect::<Vec<_>>(),
            "recommendation": {
                "action": "reindex_files",
                "paths": files_without_symbols.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
                "command": format!("reindex with path= for each affected file, or reindex force=true for full rebuild"),
                "estimated_time_seconds": files_without_symbols.len() * 2
            },
            "auto_fixable": true
        }));
    }

    // Validator 2: Orphaned symbols (symbols without files)
    #[derive(sqlx::FromRow)]
    struct OrphanedSymbol {
        id: i64,
        name: String,
        kind: String,
        file_id: i64,
    }

    let orphaned_symbols: Vec<OrphanedSymbol> = sqlx::query_as(
        r#"
        SELECT s.id, s.name, s.kind, s.file_id
        FROM symbols s
        LEFT JOIN files f ON s.file_id = f.id
        WHERE f.id IS NULL
        LIMIT 20
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    if !orphaned_symbols.is_empty() {
        issues.push(json!({
            "id": "orphaned_symbols_001",
            "severity": "critical",
            "category": "orphaned_data",
            "message": "Symbols exist without corresponding files",
            "details": {
                "description": format!("{} symbols reference non-existent files", orphaned_symbols.len()),
                "impact": "Database inconsistency, corrupted references",
                "root_cause": "Files were deleted but symbols not cleaned up",
                "examples": orphaned_symbols.iter().take(5).map(|r| format!("{}: '{}' (file_id: {})", r.kind, r.name, r.file_id)).collect::<Vec<_>>()
            },
            "affected_items": orphaned_symbols.iter().map(|r| format!("symbol: id {} '{}'", r.id, r.name)).collect::<Vec<_>>(),
            "recommendation": {
                "action": "delete_orphaned_data",
                "ids": orphaned_symbols.iter().map(|r| r.id).collect::<Vec<_>>(),
                "command": "DELETE FROM symbols WHERE file_id NOT IN (SELECT id FROM files)",
                "estimated_time_seconds": 1
            },
            "auto_fixable": true
        }));
    }

    // Validator 3: Files with failed indexing status
    #[derive(sqlx::FromRow)]
    struct FailedFile {
        path: String,
    }

    let failed_files: Vec<FailedFile> = sqlx::query_as(
        r#"
        SELECT path
        FROM files
        WHERE indexing_status = 'failed'
        LIMIT 20
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    if !failed_files.is_empty() {
        issues.push(json!({
            "id": "failed_indexing_001",
            "severity": "critical",
            "category": "failed_operation",
            "message": "Files failed to index",
            "details": {
                "description": format!("{} files have failed indexing status", failed_files.len()),
                "impact": "These files are not searchable and their symbols are missing",
                "root_cause": "Parsing errors, file access issues, or bugs in indexer",
                "examples": failed_files.iter().take(5).map(|r| r.path.to_string()).collect::<Vec<_>>()
            },
            "affected_items": failed_files.iter().map(|r| format!("file: {}", r.path)).collect::<Vec<_>>(),
            "recommendation": {
                "action": "reindex_files",
                "paths": failed_files.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
                "command": "reindex path=<file> for each failed file",
                "estimated_time_seconds": failed_files.len() * 3
            },
            "auto_fixable": true
        }));
    }

    // Validator 4: Broken references
    #[derive(sqlx::FromRow)]
    #[allow(dead_code)]
    struct BrokenRef {
        id: i64,
        symbol_id: i64,
        file_id: i64,
    }

    let broken_refs: Vec<BrokenRef> = sqlx::query_as(
        r#"
        SELECT r.id, r.symbol_id, r.file_id
        FROM references r
        LEFT JOIN files f ON r.file_id = f.id
        WHERE f.id IS NULL
        LIMIT 20
        "#,
    )
    .fetch_all(ctx.sqlite.pool())
    .await?;

    if !broken_refs.is_empty() {
        issues.push(json!({
            "id": "broken_references_001",
            "severity": "high",
            "category": "broken_references",
            "message": "References point to non-existent files",
            "details": {
                "description": format!("{} references have broken file links", broken_refs.len()),
                "impact": "get_references and get_callers may return invalid results",
                "root_cause": "Files were deleted but references not cleaned up"
            },
            "affected_items": broken_refs.iter().map(|r| format!("reference: id {}", r.id)).collect::<Vec<_>>(),
            "recommendation": {
                "action": "delete_orphaned_data",
                "command": "DELETE FROM references WHERE file_id NOT IN (SELECT id FROM files)",
                "estimated_time_seconds": 1
            },
            "auto_fixable": true
        }));
    }

    // Validator 5: Database integrity
    let integrity_ok = match ctx.sqlite.check_integrity().await {
        Ok(_) => true,
        Err(e) => {
            issues.push(json!({
                "id": "database_integrity_001",
                "severity": "critical",
                "category": "corrupted_data",
                "message": "Database integrity check failed",
                "details": {
                    "description": format!("SQLite PRAGMA integrity_check failed: {}", e),
                    "impact": "Database may be corrupted, risk of data loss",
                    "root_cause": "Disk errors, interrupted writes, or software bugs"
                },
                "affected_items": ["database: gofer.db"],
                "recommendation": {
                    "action": "rebuild_index",
                    "command": "Backup current DB, delete, and run full reindex",
                    "estimated_time_seconds": 600
                },
                "auto_fixable": false
            }));
            false
        }
    };

    // Validator 6: Missing or low embeddings
    let file_count = ctx.sqlite.get_file_count().await?;
    let chunk_count = { ctx.lance.count().await.unwrap_or(0) };

    if file_count > 0 && chunk_count == 0 {
        issues.push(json!({
            "id": "missing_embeddings_001",
            "severity": "critical",
            "category": "missing_data",
            "message": "No embeddings generated despite indexed files",
            "details": {
                "description": format!("{} files indexed but 0 embedding chunks", file_count),
                "impact": "Semantic search will not work at all",
                "root_cause": "Embedder failure, LanceDB connection issues, or indexing incomplete",
                "files_count": file_count,
                "chunks_count": 0
            },
            "affected_items": ["embeddings: all files"],
            "recommendation": {
                "action": "rebuild_index",
                "command": "reindex force=true",
                "estimated_time_seconds": file_count as u64 * 2
            },
            "auto_fixable": true
        }));
    } else if file_count > 10 && chunk_count > 0 {
        let ratio = chunk_count as f64 / file_count as f64;
        if ratio < 1.0 {
            issues.push(json!({
                "id": "low_embedding_ratio_001",
                "severity": "medium",
                "category": "inconsistent_data",
                "message": "Lower than expected chunk-to-file ratio",
                "details": {
                    "description": format!("Only {:.2} chunks per file (expected 5-20)", ratio),
                    "impact": "Search quality may be reduced for some files",
                    "root_cause": "Small files, embedding failures, or incomplete indexing",
                    "files_count": file_count,
                    "chunks_count": chunk_count,
                    "ratio": ratio
                },
                "affected_items": [],
                "recommendation": {
                    "action": "reindex_files",
                    "command": "Run validate_index with scope=embeddings for details",
                    "estimated_time_seconds": 30
                },
                "auto_fixable": false
            }));
        }
    }

    // Validator 7: Outdated files (not indexed recently)
    let stale_threshold_days = 30;
    let stale_files: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM files
        WHERE last_indexed_at IS NOT NULL
        AND julianday('now') - julianday(last_indexed_at) > ?
        "#,
    )
    .bind(stale_threshold_days)
    .fetch_one(ctx.sqlite.pool())
    .await?;

    if stale_files > 0 && stale_files > file_count / 10 {
        issues.push(json!({
            "id": "stale_files_001",
            "severity": "medium",
            "category": "outdated_data",
            "message": format!("{} files not indexed in {} days", stale_files, stale_threshold_days),
            "details": {
                "description": format!("{}% of files have stale index data", (stale_files * 100 / file_count)),
                "impact": "Index may not reflect recent code changes",
                "root_cause": "Files changed but watcher didn't trigger reindex"
            },
            "affected_items": [],
            "recommendation": {
                "action": "reindex_files",
                "command": "reindex force=true",
                "estimated_time_seconds": stale_files as u64 * 2
            },
            "auto_fixable": true
        }));
    }

    let is_valid = issues.is_empty();
    let elapsed_ms = start.elapsed().as_millis();

    // Summary by severity
    let mut severity_counts = std::collections::HashMap::new();
    for issue in &issues {
        if let Some(severity) = issue.get("severity").and_then(|v| v.as_str()) {
            *severity_counts.entry(severity).or_insert(0) += 1;
        }
    }

    let summary_recommendation = if is_valid {
        "Index is healthy and consistent. No issues found."
    } else if severity_counts.get("critical").unwrap_or(&0) > &0 {
        "Critical issues found. Immediate action required. See recommendations."
    } else if severity_counts.get("high").unwrap_or(&0) > &0 {
        "High severity issues found. Address soon to maintain index quality."
    } else {
        "Minor issues found. Address when convenient."
    };

    Ok(json!({
        "valid": is_valid,
        "issues_found": issues.len(),
        "severity_breakdown": severity_counts,
        "issues": issues,
        "integrity_check": if integrity_ok { "passed" } else { "failed" },
        "summary": summary_recommendation,
        "validation_time_ms": elapsed_ms
    }))
}

/// Reindex one file, or force a full clear + pipeline resync + ref resolution.
pub async fn tool_reindex(args: Value, ctx: &ToolContext) -> Result<Value> {
    use std::time::Instant;
    use tokio_util::sync::CancellationToken;

    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    let path = args.get("path").and_then(|v| v.as_str());
    let start = Instant::now();

    let indexer = IndexerService::new(
        (*ctx.sqlite).clone(),
        Arc::clone(&ctx.lance),
        Arc::clone(&ctx.embedder),
        num_cpus::get().clamp(1, 4),
    )
    .with_cache(Arc::clone(&ctx.cache));

    if force {
        let pool = ctx.sqlite.pool();
        sqlx::query("DELETE FROM symbol_references")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM symbols").execute(pool).await?;
        sqlx::query("DELETE FROM files").execute(pool).await?;
        let _ = sqlx::query("DELETE FROM dependency_usage").execute(pool).await;

        // Wipe vector store so orphan embeddings cannot survive the rebuild.
        ctx.lance
            .clear_all()
            .await
            .map_err(|e| GoferError::ToolError(format!("lance clear failed: {}", e)))?;

        // Full disk → index rebuild (parse + embed + write).
        indexer
            .full_sync(
                ctx.root_path.as_path(),
                &[],
                None,
                None,
                CancellationToken::new(),
            )
            .await
            .map_err(|e| GoferError::ToolError(format!("full reindex failed: {}", e)))?;

        let resolved = ctx.sqlite.resolve_references().await.unwrap_or(0);
        let _ = ctx
            .sqlite
            .set_index_meta(
                "last_full_sync",
                &chrono::Utc::now().to_rfc3339(),
            )
            .await;
        ctx.cache.invalidate_all_searches().await;

        let file_count = ctx.sqlite.get_file_count().await.unwrap_or(0);
        let chunks = ctx.lance.count().await.unwrap_or(0);
        return Ok(json!({
            "ok": true,
            "mode": "force_full",
            "files_indexed": file_count,
            "chunks": chunks,
            "refs_resolved": resolved,
            "duration_ms": start.elapsed().as_millis(),
            "message": "Cleared SQLite + Lance, full_sync, resolved references.",
        }));
    }

    let Some(rel) = path else {
        return Err(GoferError::InvalidParams(
            "Provide path= for single-file reindex, or force=true for full rebuild".into(),
        )
        .into());
    };

    let abs = resolve_path(&ctx.root_path, rel);
    indexer
        .index_file(Path::new(&abs))
        .await
        .map_err(|e| GoferError::ToolError(format!("reindex failed: {}", e)))?;

    let resolved = ctx.sqlite.resolve_references().await.unwrap_or(0);
    ctx.cache.invalidate_all_searches().await;
    Ok(json!({
        "ok": true,
        "mode": "file",
        "path": rel,
        "refs_resolved": resolved,
        "duration_ms": start.elapsed().as_millis(),
        "message": format!("Reindexed {}", rel),
    }))
}
