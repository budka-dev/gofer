use super::common::{make_relative, resolve_path, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};
use walkdir::{DirEntry, WalkDir};

pub async fn tool_project_tree(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let pattern = args.get("pattern").and_then(|v| v.as_str());

    let root_path = if path.is_empty() {
        ctx.root_path.as_ref().clone()
    } else {
        ctx.root_path.join(path)
    };

    if !root_path.exists() {
        return Err(GoferError::InvalidParams(format!("Path not found: {}", path)).into());
    }

    let tree = tokio::task::spawn_blocking({
        let root_path = root_path.clone();
        let ctx_root_path = ctx.root_path.clone();
        let pattern_string = pattern.map(|s| s.to_string());

        move || {
            let mut tree = Vec::new();
            let walker = WalkDir::new(&root_path)
                .max_depth(depth)
                .sort_by_file_name()
                .into_iter();

            // Optional glob filter
            let glob_pat = pattern_string.and_then(|p| glob::Pattern::new(&p).ok());

            // We filter entries but need to iterate to get them
            for e in walker
                .filter_entry(|e: &DirEntry| {
                    let name = e.file_name().to_string_lossy();
                    // Skip hidden and common ignored dirs
                    !name.starts_with('.')
                        && name != "node_modules"
                        && name != "target"
                        && name != "dist"
                        && name != "build"
                })
                .flatten()
            {
                let relative = make_relative(&ctx_root_path, e.path().to_str().unwrap_or(""));
                if relative.is_empty() {
                    continue;
                } // skip root itself if empty

                // Apply pattern filter only to files, or inclusion logic
                if let Some(ref gp) = glob_pat {
                    if e.file_type().is_file() && !gp.matches_path(e.path()) {
                        continue;
                    }
                }

                tree.push(if e.file_type().is_dir() {
                    format!("{}/", relative)
                } else {
                    relative
                });
            }
            Ok::<_, String>(tree)
        }
    })
    .await
    .map_err(|e| GoferError::Internal(anyhow::anyhow!("Task panic: {}", e)))?
    .map_err(|e| GoferError::Internal(anyhow::anyhow!(e)))?;

    Ok(json!({
        "root": path,
        "files": tree
    }))
}

pub async fn tool_get_dependencies(args: Value, ctx: &ToolContext) -> Result<Value> {
    let ecosystem = args.get("ecosystem").and_then(|v| v.as_str());

    let deps = &ctx.sqlite.get_dependencies_filtered(ecosystem).await?;

    Ok(json!({
        "total": deps.len(),
        "dependencies": deps.iter().map(|dep| {
            if dep.dev_only == 1 {
                format!("{}@{} ({}) [dev]", dep.name, dep.version, dep.ecosystem)
            } else {
                format!("{}@{} ({})", dep.name, dep.version, dep.ecosystem)
            }
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_dependency_impact(args: Value, ctx: &ToolContext) -> Result<Value> {
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");

    if name.is_empty() {
        return Err(GoferError::InvalidParams("Dependency name is required".into()).into());
    }

    let usages = &ctx.sqlite.get_dependency_usage(name).await?;

    Ok(json!({
        "dependency": name,
        "total": usages.len(),
        "usages": usages.iter().map(|u| {
            format!("{}:{} ({})", make_relative(&ctx.root_path, &u.file_path), u.line, u.usage_type)
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_domain_stats(ctx: &ToolContext) -> Result<Value> {
    let stats = &ctx.sqlite.get_domain_stats().await?;

    let mut domains_map = serde_json::Map::new();
    for (domain, count) in stats {
        domains_map.insert(domain.clone(), json!(count));
    }

    Ok(json!({
        "domains": Value::Object(domains_map)
    }))
}

#[allow(dead_code)]
pub async fn tool_get_api_routes(args: Value, ctx: &ToolContext) -> Result<Value> {
    let side = args.get("side").and_then(|v| v.as_str());

    let mut backend_routes = Vec::new();
    let mut frontend_calls = Vec::new();

    if side.is_none() || side == Some("backend") {
        let endpoints = &ctx.sqlite.get_api_endpoints().await?;
        backend_routes = endpoints
            .iter()
            .map(|ep| format!("{} {}", ep.method, ep.path))
            .collect();
    }

    if side.is_none() || side == Some("frontend") {
        let calls = &ctx.sqlite.get_frontend_api_calls().await?;
        frontend_calls = calls
            .iter()
            .map(|call| format!("{} {}", call.method.as_deref().unwrap_or("GET"), call.path))
            .collect();
    }

    Ok(json!({
        "backend": backend_routes,
        "frontend": frontend_calls
    }))
}

pub async fn tool_get_vue_tree(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }

    match &ctx
        .sqlite
        .get_vue_tree(&resolve_path(&ctx.root_path, file))
        .await?
    {
        Some(tree) => Ok(json!({
            "file": file,
            "tree": tree.tree_text
        })),
        None => Ok(json!({
            "file": file,
            "tree": null,
            "message": format!("No Vue tree found for '{}'. File may not be indexed.", file)
        })),
    }
}

