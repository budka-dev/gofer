//! MCP tools for LSP server integration.

use anyhow::Result;
use serde_json::{json, Value};

use crate::daemon::handlers::common::ToolContext;

/// Tool: rust_goto_definition
/// Go to definition for symbol at position.
#[allow(dead_code)]
pub async fn tool_lsp_goto_definition(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    // Ensure file is opened in LSP server
    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let locations = client
        .goto_definition(std::path::Path::new(&abs_path), line, character)
        .await?;

    let results: Vec<_> = locations
        .into_iter()
        .map(|loc| {
            format!(
                "{}:{}:{}-{}:{}",
                loc.uri.path(),
                loc.range.start.line,
                loc.range.start.character,
                loc.range.end.line,
                loc.range.end.character
            )
        })
        .collect();

    Ok(json!({ "definitions": results }))
}

/// Tool: rust_find_references
/// Find all references to symbol at position.
#[allow(dead_code)]
pub async fn tool_lsp_find_references(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;
    let include_declaration = args["include_declaration"].as_bool().unwrap_or(true);

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let locations = client
        .find_references(
            std::path::Path::new(&abs_path),
            line,
            character,
            include_declaration,
        )
        .await?;

    let results: Vec<_> = locations
        .into_iter()
        .map(|loc| {
            format!(
                "{}:{}:{}-{}:{}",
                loc.uri.path(),
                loc.range.start.line,
                loc.range.start.character,
                loc.range.end.line,
                loc.range.end.character
            )
        })
        .collect();

    Ok(json!({ "references": results }))
}

/// Tool: rust_hover
/// Get hover information (type, docs) for symbol at position.
#[allow(dead_code)]
pub async fn tool_lsp_hover(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let hover = client
        .hover(std::path::Path::new(&abs_path), line, character)
        .await?;

    match hover {
        Some(h) => {
            let content = match h.contents {
                lsp_types::HoverContents::Scalar(marked) => match marked {
                    lsp_types::MarkedString::String(s) => s,
                    lsp_types::MarkedString::LanguageString(ls) => {
                        format!("```{}\n{}\n```", ls.language, ls.value)
                    }
                },
                lsp_types::HoverContents::Array(arr) => arr
                    .into_iter()
                    .map(|marked| match marked {
                        lsp_types::MarkedString::String(s) => s,
                        lsp_types::MarkedString::LanguageString(ls) => {
                            format!("```{}\n{}\n```", ls.language, ls.value)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                lsp_types::HoverContents::Markup(markup) => markup.value,
            };

            Ok(json!({
                "content": content,
                "range": h.range.map(|r| json!({
                    "start_line": r.start.line,
                    "start_character": r.start.character,
                    "end_line": r.end.line,
                    "end_character": r.end.character,
                }))
            }))
        }
        None => Ok(json!({ "content": null })),
    }
}

/// Tool: rust_diagnostics
/// Get compiler diagnostics (errors, warnings) for a file.
#[allow(dead_code)]
pub async fn tool_lsp_diagnostics(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let diagnostics = client
        .diagnostics(std::path::Path::new(&abs_path))
        .await?;

    let results: Vec<_> = diagnostics
        .into_iter()
        .map(|diag| {
            let sev = match diag.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) => "error",
                Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
                Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
                _ => "unknown",
            };
            let code = diag
                .code
                .map(|c| match c {
                    lsp_types::NumberOrString::Number(n) => n.to_string(),
                    lsp_types::NumberOrString::String(s) => s,
                })
                .map(|c| format!(" {}", c))
                .unwrap_or_default();

            format!(
                "{}:{}-{}:{} [{}]{} {} ({})",
                diag.range.start.line + 1,
                diag.range.start.character + 1,
                diag.range.end.line + 1,
                diag.range.end.character + 1,
                sev,
                code,
                diag.message,
                diag.source.unwrap_or_default()
            )
        })
        .collect();

    Ok(json!({ "diagnostics": results }))
}

/// Tool: rust_completions
/// Get code completions for Rust at position.
#[allow(dead_code)]
pub async fn tool_lsp_completions(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let items = client
        .completions(std::path::Path::new(&abs_path), line, character)
        .await?;

    let results: Vec<_> = items
        .into_iter()
        .map(|item| {
            let detail = item.detail.map(|d| format!(" - {}", d)).unwrap_or_default();
            format!("{} ({:?}){}", item.label, item.kind, detail)
        })
        .collect();

    Ok(json!({ "completions": results }))
}

/// Tool: rust_inlay_hints
/// Get inlay hints (type annotations, parameter names) for a file range.
#[allow(dead_code)]
pub async fn tool_lsp_inlay_hints(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let start_line = args["start_line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing start_line"))? as u32;
    let end_line = args["end_line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing end_line"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let hints = client
        .inlay_hints(std::path::Path::new(&abs_path), start_line, end_line)
        .await?;

    let results: Vec<_> = hints
        .into_iter()
        .map(|hint| {
            let label = match hint.label {
                lsp_types::InlayHintLabel::String(s) => s,
                lsp_types::InlayHintLabel::LabelParts(parts) => parts
                    .into_iter()
                    .map(|p| p.value)
                    .collect::<Vec<_>>()
                    .join(""),
            };

            format!(
                "{}:{} [{:?}] {}",
                hint.position.line + 1,
                hint.position.character + 1,
                hint.kind,
                label
            )
        })
        .collect();

    Ok(json!({ "hints": results }))
}

/// Tool: rust_code_actions
/// Get code actions (quick fixes, refactorings) for a file range.
#[allow(dead_code)]
pub async fn tool_lsp_code_actions(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let start_line = args["start_line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing start_line"))? as u32;
    let end_line = args["end_line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing end_line"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    // Get diagnostics for this range to pass as context
    let all_diagnostics = client
        .diagnostics(std::path::Path::new(&abs_path))
        .await?;

    let actions = client
        .code_actions(
            std::path::Path::new(&abs_path),
            start_line,
            end_line,
            all_diagnostics,
        )
        .await?;

    let results: Vec<_> = actions
        .into_iter()
        .map(|action| match action {
            lsp_types::CodeActionOrCommand::CodeAction(ca) => {
                let kind = ca
                    .kind
                    .map(|k| format!(" [{}]", k.as_str()))
                    .unwrap_or_default();
                let pref = if ca.is_preferred.unwrap_or(false) {
                    " (preferred)"
                } else {
                    ""
                };
                format!("CodeAction: {}{}{}", ca.title, kind, pref)
            }
            lsp_types::CodeActionOrCommand::Command(cmd) => {
                format!("Command: {}", cmd.title)
            }
        })
        .collect();

    Ok(json!({ "actions": results }))
}


/// Tool: rust_document_symbols
/// Get document outline (structures, functions, enums, traits) for a file.
#[allow(dead_code)]
pub async fn tool_lsp_document_symbols(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let symbols = client
        .document_symbols(std::path::Path::new(&abs_path))
        .await?;

    fn format_symbol(sym: &lsp_types::DocumentSymbol, depth: usize) -> String {
        let indent = "  ".repeat(depth);
        let mut result = format!(
            "{}{} [{:?}] {} (line {})",
            indent,
            sym.name,
            sym.kind,
            sym.detail.as_deref().unwrap_or(""),
            sym.range.start.line + 1
        );
        if let Some(ref children) = sym.children {
            for child in children {
                result.push('\n');
                result.push_str(&format_symbol(child, depth + 1));
            }
        }
        result
    }

    let results: Vec<_> = symbols
        .into_iter()
        .map(|sym| format_symbol(&sym, 0))
        .collect();

    Ok(json!({ "symbols": results }))
}

/// Tool: rust_workspace_symbols
/// Search for symbols across the entire workspace by name.
#[allow(dead_code)]
pub async fn tool_lsp_workspace_symbols(args: Value, ctx: &ToolContext) -> Result<Value> {
    let query = args["query"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing query"))?;

    let mut by_file: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let clients = ctx.lsp_clients.read().await;

    for client in clients.values() {
        if let Ok(symbols) = client.workspace_symbols(query).await {
            for sym in symbols {
                let file = sym.location.uri.path().to_string();
                let container = sym
                    .container_name
                    .map(|c| format!(" in {}", c))
                    .unwrap_or_default();
                let s = format!(
                    "{}: [{:?}] {}{}",
                    sym.location.range.start.line + 1,
                    sym.kind,
                    sym.name,
                    container
                );
                by_file.entry(file).or_default().push(s);
            }
        }
    }

    Ok(json!({ "symbols": by_file }))
}

/// Tool: rust_goto_implementation
/// Go to concrete implementation(s) of a trait method or type.
#[allow(dead_code)]
pub async fn tool_lsp_goto_implementation(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let locations = client
        .goto_implementation(std::path::Path::new(&abs_path), line, character)
        .await?;

    let results: Vec<_> = locations
        .into_iter()
        .map(|loc| {
            format!(
                "{}:{}:{}-{}:{}",
                loc.uri.path(),
                loc.range.start.line + 1,
                loc.range.start.character + 1,
                loc.range.end.line + 1,
                loc.range.end.character + 1
            )
        })
        .collect();

    Ok(json!({ "implementations": results }))
}

/// Tool: rust_rename
/// Rename a symbol across the entire workspace (safe semantic rename).
#[allow(dead_code)]
pub async fn tool_lsp_rename(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;
    let new_name = args["new_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing new_name"))?;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let workspace_edit = client
        .rename(std::path::Path::new(&abs_path), line, character, new_name)
        .await?;

    match workspace_edit {
        Some(edit) => {
            let mut changes = Vec::new();

            if let Some(document_changes) = edit.document_changes {
                match document_changes {
                    lsp_types::DocumentChanges::Edits(edits) => {
                        for edit in edits {
                            changes.push(json!({
                                "file": edit.text_document.uri.path().as_str(),
                                "edits": edit.edits.into_iter().map(|e| {
                                    let text = match e {
                                        lsp_types::OneOf::Left(te) => te.new_text,
                                        lsp_types::OneOf::Right(ae) => ae.text_edit.new_text,
                                    };
                                    json!({ "new_text": text })
                                }).collect::<Vec<_>>()
                            }));
                        }
                    }
                    lsp_types::DocumentChanges::Operations(ops) => {
                        for op in ops {
                            if let lsp_types::DocumentChangeOperation::Edit(edit) = op {
                                changes.push(json!({
                                    "file": edit.text_document.uri.path().as_str(),
                                    "edits": edit.edits.len()
                                }));
                            }
                        }
                    }
                }
            } else if let Some(change_map) = edit.changes {
                for (uri, edits) in change_map {
                    changes.push(json!({
                        "file": uri.path().as_str(),
                        "edits": edits.len()
                    }));
                }
            }

            Ok(json!({
                "success": true,
                "changes": changes,
                "total_files": changes.len()
            }))
        }
        None => Ok(json!({
            "success": false,
            "message": "No rename available at this position"
        })),
    }
}

/// Tool: rust_expand_macro
/// Expand macro at position to see generated code.
#[allow(dead_code)]
pub async fn tool_lsp_expand_macro(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let expansion = client
        .expand_macro(std::path::Path::new(&abs_path), line, character)
        .await?;

    match expansion {
        Some(exp) => Ok(json!({
            "success": true,
            "expansion": exp
        })),
        None => Ok(json!({
            "success": false,
            "message": "No macro at this position"
        })),
    }
}

/// Tool: rust_incoming_calls
/// Get incoming calls (callers) for a function/method at position.
#[allow(dead_code)]
pub async fn tool_lsp_incoming_calls(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let items = client
        .prepare_call_hierarchy(std::path::Path::new(&abs_path), line, character)
        .await?;

    if items.is_empty() {
        return Ok(json!({ "calls": [] }));
    }

    let incoming = client.incoming_calls(items[0].clone()).await?;

    let results: Vec<_> = incoming
        .into_iter()
        .map(|call| {
            let ranges: Vec<String> = call
                .from_ranges
                .iter()
                .map(|r| format!("{}:{}", r.start.line + 1, r.start.character + 1))
                .collect();
            format!(
                "{}:{}:{} [{:?}] {} (from ranges: {})",
                call.from.uri.path(),
                call.from.range.start.line + 1,
                call.from.range.start.character + 1,
                call.from.kind,
                call.from.name,
                ranges.join(", ")
            )
        })
        .collect();

    Ok(json!({ "calls": results }))
}

/// Tool: rust_outgoing_calls
/// Get outgoing calls (callees) for a function/method at position.
#[allow(dead_code)]
pub async fn tool_lsp_outgoing_calls(args: Value, ctx: &ToolContext) -> Result<Value> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
    let line = args["line"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing line"))? as u32;
    let character = args["character"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing character"))? as u32;

    let client = ctx.get_lsp_client(file_path).await?.ok_or_else(|| anyhow::anyhow!("No LSP client configured for this file type"))?;

    let abs_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        ctx.root_path.join(file_path).to_string_lossy().to_string()
    };

    let content = tokio::fs::read_to_string(&abs_path).await?;
    client
        .did_open(std::path::Path::new(&abs_path), content)
        .await?;

    let items = client
        .prepare_call_hierarchy(std::path::Path::new(&abs_path), line, character)
        .await?;

    if items.is_empty() {
        return Ok(json!({ "calls": [] }));
    }

    let outgoing = client.outgoing_calls(items[0].clone()).await?;

    let results: Vec<_> = outgoing
        .into_iter()
        .map(|call| {
            let ranges: Vec<String> = call
                .from_ranges
                .iter()
                .map(|r| format!("{}:{}", r.start.line + 1, r.start.character + 1))
                .collect();
            format!(
                "{}:{}:{} [{:?}] {} (from ranges: {})",
                call.to.uri.path(),
                call.to.range.start.line + 1,
                call.to.range.start.character + 1,
                call.to.kind,
                call.to.name,
                ranges.join(", ")
            )
        })
        .collect();

    Ok(json!({ "calls": results }))
}
