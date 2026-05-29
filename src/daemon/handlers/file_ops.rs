//! File operations tools - Phase 1 implementation
//!
//! Implements:
//! - list_directory - recursive directory listing
//! - read_file_chunk - read file by line range (extends existing tool_read_file)
//! - patch_file - search & replace in files
//! - write_file - create/overwrite files
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

/// Patch file with search & replace
pub async fn tool_patch_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let search_string = args
        .get("search_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("search_string is required".into()))?;

    let replace_string = args
        .get("replace_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("replace_string is required".into()))?;

    let occurrence = args.get("occurrence").and_then(|v| v.as_u64()).unwrap_or(1) as usize;

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    // Read file content
    let content = tokio::fs::read_to_string(&abs_path).await?;

    // Find all occurrences
    let occurrences: Vec<_> = content.match_indices(search_string).collect();
    let occurrences_found = occurrences.len();

    if occurrences_found == 0 {
        return Ok(json!({
            "path": path,
            "status": "error",
            "error": "search_string_not_found",
            "message": format!("Could not find '{}' in {}", search_string, path),
            "occurrences_found": 0,
        }));
    }

    // Check for ambiguous match
    if occurrence > 0 && occurrence > occurrences_found {
        return Ok(json!({
            "path": path,
            "status": "error",
            "error": "invalid_occurrence",
            "message": format!(
                "Occurrence {} requested but only {} occurrences found",
                occurrence, occurrences_found
            ),
            "occurrences_found": occurrences_found,
        }));
    }

    // Replace based on occurrence parameter
    let new_content = if occurrence == 0 {
        // Replace all occurrences
        content.replace(search_string, replace_string)
    } else {
        // Replace specific occurrence (1-indexed)
        let idx = occurrence - 1;
        let (start_pos, _) = occurrences[idx];

        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..start_pos]);
        result.push_str(replace_string);
        result.push_str(&content[start_pos + search_string.len()..]);
        result
    };

    // Write back to file
    tokio::fs::write(&abs_path, &new_content).await?;

    // Invalidate cache
    ctx.cache.invalidate_file(path).await;

    // Calculate which lines were changed
    let lines_changed = calculate_changed_lines(&content, &new_content);

    Ok(json!({
        "path": path,
        "status": "patched",
        "occurrences_found": occurrences_found,
        "occurrences_replaced": if occurrence == 0 { occurrences_found } else { 1 },
        "lines_changed": lines_changed,
    }))
}

/// Write file (create or overwrite)
pub async fn tool_write_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("content is required".into()))?;

    let create_dirs = args
        .get("create_dirs")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;
    let existed = abs_path.exists();

    // Create parent directories if requested
    if create_dirs {
        if let Some(parent) = abs_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // Write content
    tokio::fs::write(&abs_path, content).await?;

    // Invalidate cache
    ctx.cache.invalidate_file(path).await;

    let lines = content.lines().count();
    let size_bytes = content.len() as u64;

    Ok(json!({
        "path": path,
        "action": if existed { "overwritten" } else { "created" },
        "size": format_bytes(size_bytes),
        "lines": lines,
    }))
}

/// Append to file
pub async fn tool_append_to_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("content is required".into()))?;

    let newline_before = args
        .get("newline_before")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    // Read existing content to count lines
    let existing = tokio::fs::read_to_string(&abs_path).await?;
    let old_line_count = existing.lines().count();

    // Prepare content to append
    let to_append = if newline_before && !existing.ends_with('\n') {
        format!("\n{}", content)
    } else {
        content.to_string()
    };

    // Append
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&abs_path)
        .await?;
    file.write_all(to_append.as_bytes()).await?;

    // Invalidate cache
    ctx.cache.invalidate_file(path).await;

    let lines_added = to_append.lines().count();
    let bytes_added = to_append.len() as u64;

    Ok(json!({
        "path": path,
        "status": "appended",
        "bytes_added": bytes_added,
        "lines_added": lines_added,
        "new_total_lines": old_line_count + lines_added,
    }))
}

/// Create directory
pub async fn tool_create_directory(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let recursive = args
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;
    let already_existed = abs_path.exists();

    if already_existed {
        return Ok(json!({
            "path": path,
            "status": "already_exists",
            "already_existed": true,
        }));
    }

    if recursive {
        tokio::fs::create_dir_all(&abs_path).await?;
    } else {
        tokio::fs::create_dir(&abs_path).await?;
    }

    Ok(json!({
        "path": path,
        "status": "created",
        "already_existed": false,
    }))
}

/// Move file or directory
pub async fn tool_move_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("source is required".into()))?;

    let destination = args
        .get("destination")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("destination is required".into()))?;

    let overwrite = args
        .get("overwrite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let abs_source = resolve_path_buf(&ctx.root_path, source)?;
    let abs_dest = resolve_path_buf(&ctx.root_path, destination)?;

    if !abs_source.exists() {
        return Err(GoferError::InvalidParams(format!("Source not found: {}", source)).into());
    }

    if abs_dest.exists() && !overwrite {
        return Ok(json!({
            "status": "conflict",
            "message": format!("Destination already exists: {}", destination),
            "source": source,
            "destination": destination,
        }));
    }

    // Create parent directory if needed
    if let Some(parent) = abs_dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Move/rename
    tokio::fs::rename(&abs_source, &abs_dest).await?;

    // Invalidate caches
    ctx.cache.invalidate_file(source).await;
    ctx.cache.invalidate_file(destination).await;

    Ok(json!({
        "source": source,
        "destination": destination,
        "status": "moved",
        "type": if abs_dest.is_dir() { "directory" } else { "file" },
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

fn calculate_changed_lines(old: &str, new: &str) -> Vec<usize> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut changed = Vec::new();

    for (idx, (old_line, new_line)) in old_lines.iter().zip(new_lines.iter()).enumerate() {
        if old_line != new_line {
            changed.push(idx + 1); // 1-indexed
        }
    }

    // Handle case where line count changed
    if old_lines.len() != new_lines.len() {
        let min_len = old_lines.len().min(new_lines.len());
        for idx in min_len..old_lines.len().max(new_lines.len()) {
            changed.push(idx + 1);
        }
    }

    changed
}
