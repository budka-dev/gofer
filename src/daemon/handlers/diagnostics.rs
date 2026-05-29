use super::common::{make_relative, resolve_path, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};

pub async fn tool_get_errors(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str());
    let severity = args.get("severity").and_then(|v| v.as_str());
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(500) as u32;

    let resolved_file = file.map(|f| resolve_path(&ctx.root_path, f));
    let errors = &ctx
        .sqlite
        .get_errors(resolved_file.as_deref(), severity, offset, limit)
        .await?;
    let count = errors.len() as u32;

    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for err in errors {
        let path = make_relative(&ctx.root_path, &err.file_path);
        let entry = if let Some(ref sugg) = err.suggestion {
            format!(
                "{}:{} [{}] {}: {} (Suggestion: {})",
                err.line,
                err.column.unwrap_or(0),
                err.severity,
                err.code.as_deref().unwrap_or(""),
                err.message,
                sugg
            )
        } else {
            format!(
                "{}:{} [{}] {}: {}",
                err.line,
                err.column.unwrap_or(0),
                err.severity,
                err.code.as_deref().unwrap_or(""),
                err.message
            )
        };
        map.entry(path).or_default().push(entry);
    }

    Ok(json!({
        "total": count,
        "offset": offset,
        "limit": limit,
        "has_more": count == limit,
        "errors": map
    }))
}

pub async fn tool_run_diagnostics(args: Value, ctx: &ToolContext) -> Result<Value> {
    use crate::indexer::diagnostics::{self, CargoCheckOptions};

    let options = CargoCheckOptions {
        workspace: args.get("workspace").and_then(|v| v.as_bool()),
        all_targets: args.get("all_targets").and_then(|v| v.as_bool()),
        package: args
            .get("package")
            .and_then(|v| v.as_str())
            .map(String::from),
        manifest_path: args
            .get("manifest_path")
            .and_then(|v| v.as_str())
            .map(String::from),
    };

    let file_filter = args
        .get("file")
        .and_then(|v| v.as_str())
        .map(|f| resolve_path(&ctx.root_path, f));

    let result = diagnostics::run_diagnostics(&ctx.root_path, &ctx.sqlite, options).await?;

    let mut cargo_val = serde_json::to_value(&result.cargo)?;
    let mut tsc_val = serde_json::to_value(&result.tsc)?;
    let mut total_errors = result.total_errors;
    let mut total_warnings = result.total_warnings;

    if let Some(ref needle) = file_filter {
        let (filtered_cargo, c_err, c_warn) = filter_diagnostics_by_file(&cargo_val, needle);
        let (filtered_tsc, t_err, t_warn) = filter_diagnostics_by_file(&tsc_val, needle);
        cargo_val = filtered_cargo;
        tsc_val = filtered_tsc;
        total_errors = c_err + t_err;
        total_warnings = c_warn + t_warn;
    }

    Ok(json!({
        "cargo": cargo_val,
        "tsc": tsc_val,
        "total": { "errors": total_errors, "warnings": total_warnings }
    }))
}

/// Return (filtered_value, errors_count, warnings_count) for diagnostics
/// payload, keeping only entries whose `file` (or `file_path`) field contains
/// `needle`. Unknown shapes are returned as-is with zero counts.
fn filter_diagnostics_by_file(val: &Value, needle: &str) -> (Value, usize, usize) {
    let Some(obj) = val.as_object() else {
        return (val.clone(), 0, 0);
    };
    let mut out = serde_json::Map::new();
    let mut errors = 0usize;
    let mut warnings = 0usize;

    for (k, v) in obj {
        if k == "diagnostics" {
            if let Some(arr) = v.as_array() {
                let filtered: Vec<Value> = arr
                    .iter()
                    .filter(|d| {
                        let path = d
                            .get("file")
                            .or_else(|| d.get("file_path"))
                            .or_else(|| d.get("path"))
                            .and_then(|p| p.as_str())
                            .unwrap_or("");
                        path.contains(needle)
                    })
                    .cloned()
                    .collect();
                for d in &filtered {
                    match d.get("severity").and_then(|s| s.as_str()) {
                        Some("error") => errors += 1,
                        Some("warning") => warnings += 1,
                        _ => {}
                    }
                }
                out.insert(k.clone(), Value::Array(filtered));
                continue;
            }
        }
        out.insert(k.clone(), v.clone());
    }

    (Value::Object(out), errors, warnings)
}

pub async fn tool_health_check(ctx: &ToolContext) -> Result<Value> {
    use std::time::Instant;

    let mut components = Vec::new();
    let mut all_healthy = true;

    // 1. SQLite check — simple query
    let sqlite_start = Instant::now();
    let sqlite_status = match ctx.sqlite.get_file_count().await {
        Ok(count) => {
            json!({
                "name": "sqlite",
                "status": "healthy",
                "latency_ms": sqlite_start.elapsed().as_millis(),
                "details": { "indexed_files": count }
            })
        }
        Err(e) => {
            all_healthy = false;
            tracing::error!("Health check: SQLite failed: {}", e);
            json!({
                "name": "sqlite",
                "status": "unhealthy",
                "error": e.to_string()
            })
        }
    };
    components.push(sqlite_status);

    // 2. LanceDB check — verify table exists and can be queried
    let lance_start = Instant::now();
    let lance_status = {
        match ctx.lance.count().await {
            Ok(count) => {
                json!({
                    "name": "lancedb",
                    "status": "healthy",
                    "latency_ms": lance_start.elapsed().as_millis(),
                    "details": { "chunk_count": count }
                })
            }
            Err(e) => {
                all_healthy = false;
                tracing::error!("Health check: LanceDB failed: {}", e);
                json!({
                    "name": "lancedb",
                    "status": "unhealthy",
                    "error": e.to_string()
                })
            }
        }
    };
    components.push(lance_status);

    // 3. Embedder check — try to embed a test string
    let embed_start = Instant::now();
    let embed_status = match ctx.embedder.embed_query("health check test").await {
        Ok(embedding) => {
            json!({
                "name": "embedder",
                "status": "healthy",
                "latency_ms": embed_start.elapsed().as_millis(),
                "details": {
                    "dimension": embedding.len(),
                    "model": ctx.embedder.model_name()
                }
            })
        }
        Err(e) => {
            all_healthy = false;
            tracing::error!("Health check: Embedder failed: {}", e);
            json!({
                "name": "embedder",
                "status": "unhealthy",
                "error": e.to_string()
            })
        }
    };
    components.push(embed_status);

    Ok(json!({
        "status": if all_healthy { "healthy" } else { "unhealthy" },
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "components": components
    }))
}

pub async fn tool_get_config_keys(ctx: &ToolContext) -> Result<Value> {
    let keys = &ctx.sqlite.get_config_keys().await?;

    Ok(json!({
        "total": keys.len(),
        "keys": keys.iter().map(|key| {
            let req_str = if key.required == 1 { " [required]" } else { "" };
            format!("{} ({}) src:{}{}", key.key_name, key.data_type.as_deref().unwrap_or(""), key.source, req_str)
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_has_tests_for(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str());

    let Some(file_path) = file else {
        return Err(GoferError::InvalidParams("file parameter required".into()).into());
    };

    // Common test file patterns
    let test_patterns = vec![
        // TypeScript/JavaScript patterns
        format!("{}.test.ts", file_path.trim_end_matches(".ts")),
        format!("{}.test.js", file_path.trim_end_matches(".js")),
        format!("{}.spec.ts", file_path.trim_end_matches(".ts")),
        format!("{}.spec.js", file_path.trim_end_matches(".js")),
        // Rust patterns
        format!("{}_test.rs", file_path.trim_end_matches(".rs")),
        // Python patterns
        format!("test_{}", file_path),
        format!("{}_test.py", file_path.trim_end_matches(".py")),
        // General tests directory patterns
        format!(
            "tests/test_{}",
            file_path.split('/').next_back().unwrap_or(file_path)
        ),
        format!(
            "tests/{}_test",
            file_path.split('/').next_back().unwrap_or(file_path)
        ),
        format!(
            "__tests__/{}",
            file_path.split('/').next_back().unwrap_or(file_path)
        ),
    ];

    let mut found_tests = Vec::new();

    // We can't query SQLite easily with "LIKE" for all patterns efficiently without raw SQL access
    // exposed via SqliteStorage.
    // However, we can use `tool_find_files` logic or `glob` if we want to check disk.
    // Or assume `ctx.sqlite` has a method to find files matching pattern.
    // Since `tools.rs` implementation used `sqlx::query_scalar` directly on `ctx.sqlite.pool()`,
    // and `ToolContext` doesn't expose pool directly (it exposes `Arc<SqliteStorage>`),
    // we might need to add a method to `SqliteStorage` or use `find_files` via glob.

    // BUT `SqliteStorage` struct usually exposes `pool` if pub, or we added a method.
    // Let's assume we can use `glob` on disk for now as it's safer than raw SQL injection if pool isn't exposed.
    // Or check if `SqliteStorage` has `find_test_files` (unlikely).

    // Actually, `tool_has_tests_for` implementation I read earlier used `ctx.sqlite.pool()`.
    // This implies `pool()` is public on `SqliteStorage`.
    // If so, I need `sqlx` dependency here.

    // To avoid adding sqlx dependency to this file if not needed, I'll use `glob` approach on disk
    // which effectively answers "does a test file exist".

    for pattern in test_patterns {
        // glob pattern: *pattern*? No, exact matches usually.
        // Let's just check file existence for exact matches first.
        let path = ctx.root_path.join(&pattern);
        if path.exists() {
            found_tests.push(pattern);
        }
    }

    // Remove duplicates
    found_tests.sort();
    found_tests.dedup();

    let has_tests = !found_tests.is_empty();

    Ok(json!({
        "has_tests": has_tests,
        "file": file_path,
        "test_files": found_tests,
        "count": found_tests.len()
    }))
}
