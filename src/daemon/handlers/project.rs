use super::common::{make_relative, ToolContext};
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

