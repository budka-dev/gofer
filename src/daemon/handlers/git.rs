use super::common::ToolContext;
use crate::error::GoferError;
use crate::indexer::git::GitRepo;
use anyhow::Result;
use serde_json::{json, Value};

pub async fn tool_git_blame(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");

    // Accept either a single `line` or a `start_line`/`end_line` range. Older
    // clients passing only `line` keep working; new ones can ask for a span and
    // get one entry per line in the response. Previously the handler always
    // collapsed to a single line so range requests silently returned just line 1.
    let start_line = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("line").and_then(|v| v.as_u64()))
        .unwrap_or(1) as u32;
    let end_line = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(start_line);

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }
    if end_line < start_line {
        return Err(GoferError::InvalidParams(
            "end_line must be >= start_line".into(),
        )
        .into());
    }

    let repo = match GitRepo::open(&ctx.root_path) {
        Some(r) => r,
        None => return Err(GoferError::InvalidParams("Not a git repository".into()).into()),
    };
    let file_path = &ctx.root_path.join(file);

    let blames = repo.blame_lines(file_path, start_line, end_line);
    if blames.is_empty() {
        return Ok(json!({
            "file": file,
            "start_line": start_line,
            "end_line": end_line,
            "lines": [],
            "message": format!("No blame info for {}:{}-{}", file, start_line, end_line)
        }));
    }

    let lines: Vec<Value> = blames
        .iter()
        .map(|b| {
            json!({
                "line": b.line,
                "lines_in_hunk": b.lines_in_hunk,
                "author": b.author,
                "date": b.timestamp,
                "commit": &b.commit_id[..8.min(b.commit_id.len())],
                "message": b.message,
            })
        })
        .collect();

    Ok(json!({
        "file": file,
        "start_line": start_line,
        "end_line": end_line,
        "total": lines.len(),
        "lines": lines,
    }))
}

pub async fn tool_git_history(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    if file.is_empty() {
        return Err(GoferError::InvalidParams("File path is required".into()).into());
    }

    let repo = match GitRepo::open(&ctx.root_path) {
        Some(r) => r,
        None => return Err(GoferError::InvalidParams("Not a git repository".into()).into()),
    };
    let file_path = &ctx.root_path.join(file);
    let history = repo.file_history(file_path, limit);

    Ok(json!({
        "file": file,
        "total": history.len(),
        "commits": history.iter().map(|c| {
            format!("{} {} <{}> ({}): {}", &c.id[..8.min(c.id.len())], c.author, c.email, c.timestamp, c.message.lines().next().unwrap_or(""))
        }).collect::<Vec<_>>()
    }))
}

pub async fn tool_git_diff(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str());
    let staged = args
        .get("staged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let repo = match GitRepo::open(&ctx.root_path) {
        Some(r) => r,
        None => return Err(GoferError::InvalidParams("Not a git repository".into()).into()),
    };

    let file_path = file.map(|f| ctx.root_path.join(f));

    match repo.git_diff(file_path.as_deref(), staged) {
        Some(diff_text) => Ok(json!({
            "file": file.unwrap_or("(all)"),
            "staged": staged,
            "diff": diff_text
        })),
        None => Ok(json!({
            "file": file.unwrap_or("(all)"),
            "staged": staged,
            "diff": "",
            "message": "No changes found"
        })),
    }
}

pub async fn tool_suggest_commit(args: Value, ctx: &ToolContext) -> Result<Value> {
    let style = args
        .get("style")
        .and_then(|v| v.as_str())
        .unwrap_or("conventional");

    let include_emoji = args
        .get("include_emoji")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Use root_path as repository path
    let repo_path = &ctx.root_path;

    // Call commit analyzer
    let suggestion = crate::commit::suggest_commit_message(repo_path, include_emoji, style).await?;

    Ok(serde_json::to_value(suggestion)?)
}

pub async fn tool_verify_patch(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

    if file.is_empty() || content.is_empty() {
        return Err(
            GoferError::InvalidParams("Both 'file' and 'content' are required".into()).into(),
        );
    }

    let result = crate::indexer::diagnostics::verify_patch(&ctx.root_path, file, content).await?;

    Ok(json!({
        "file": file,
        "status": result.status,
        "summary": result.summary,
        "diagnostics": result.diagnostics.iter().map(|d| {
            let col = d.column.map(|c| format!(":{}", c)).unwrap_or_default();
            let code = d.code.as_deref().map(|c| format!("{}: ", c)).unwrap_or_default();
            let mut s = format!("{}{u} [{}] {}{}", d.line, d.severity, code, d.message, u = col);
            if let Some(ref sugg) = d.suggestion {
                s.push_str(&format!(" (suggestion: {})", sugg));
            }
            s
        }).collect::<Vec<_>>()
    }))
}
