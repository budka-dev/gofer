//! File operations tools - read-only subset
//!
//! Implements:
//! - list_directory - recursive directory listing
//! - get_file_metadata - file metadata (size, mtime, lines)

use super::common::{make_relative_pathbuf, resolve_path_buf, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};
use walkdir::WalkDir;

/// List directory structure recursively
pub async fn tool_list_directory(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let recursive = args
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let exclude_patterns = args
        .get("exclude_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![
                "node_modules".to_string(),
                "target".to_string(),
                ".git".to_string(),
                "dist".to_string(),
                "build".to_string(),
            ]
        });

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("Path not found: {}", path)).into());
    }

    if !abs_path.is_dir() {
        return Err(GoferError::InvalidParams(format!("Not a directory: {}", path)).into());
    }

    let mut total_files = 0u64;
    let mut total_size = 0u64;
    let mut entries = Vec::new();

    let walker = if recursive {
        WalkDir::new(&abs_path)
    } else {
        WalkDir::new(&abs_path).max_depth(1)
    };

    for entry in walker.into_iter().filter_entry(|e| {
        // Skip excluded patterns
        let path_str = e.path().to_string_lossy();
        !exclude_patterns
            .iter()
            .any(|pattern| path_str.contains(pattern))
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip the root directory itself
        if entry.path() == abs_path {
            continue;
        }

        let rel_path = make_relative_pathbuf(&ctx.root_path, entry.path());
        let metadata = entry.metadata().ok();
        let is_dir = entry.file_type().is_dir();

        if !is_dir {
            total_files += 1;
            if let Some(meta) = &metadata {
                total_size += meta.len();
            }
        }

        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        if is_dir {
            entries.push(format!("{}/", rel_path));
        } else {
            entries.push(format!("{} ({})", rel_path, format_bytes(size)));
        }
    }

    Ok(json!({
        "path": path,
        "entries": entries,
        "total_files": total_files,
        "total_size": total_size,
        "total_size_human": format_bytes(total_size),
    }))
}

/// Get file metadata (size, mtime, lines)
pub async fn tool_get_file_metadata(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    let metadata = tokio::fs::metadata(&abs_path).await?;
    let size_bytes = metadata.len();
    let modified = metadata.modified().ok();
    let created = metadata.created().ok();

    // Try to read file to count lines (if it's a text file)
    let mut line_count = None;
    let mut is_binary = false;

    if abs_path.is_file() {
        // Try to read as text
        if let Ok(content) = tokio::fs::read_to_string(&abs_path).await {
            line_count = Some(content.lines().count());
        } else {
            is_binary = true;
        }
    }

    Ok(json!({
        "path": path,
        "size_bytes": size_bytes,
        "size_human": format_bytes(size_bytes),
        "created_at": created.map(|t| format!("{:?}", t)),
        "modified_at": modified.map(|t| format!("{:?}", t)),
        "line_count": line_count,
        "is_binary": is_binary,
        "is_directory": abs_path.is_dir(),
    }))
}

// Helper functions

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

