//! Code Quality Tools - Phase 2 implementation
//!
//! Implements:
//! - format_file - автоформатирование (rustfmt, prettier, black, gofmt)
//! - lint_file - запуск линтера (clippy, eslint, ruff)
//! - apply_lint_fix - применение автофиксов от линтера

use super::common::{resolve_path_buf, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintWarning {
    pub line: u32,
    pub column: u32,
    pub severity: String,
    pub message: String,
    pub code: String,
    pub fix_available: bool,
}

/// Inspect the project root to figure out which JS/TS linter the project
/// actually uses. Walks up from `root` so subprojects inherit. Returns:
/// * `eslint` if any eslint config exists or it's in package.json deps
/// * `biome` if biome.json[c] is present or @biomejs/biome is in deps
/// * `oxlint` if oxlint.config.* or oxlint is in deps
/// * `None` for non-JS extensions or when nothing matches.
///
/// Without this, every TS file got linted with the manifest default (biome),
/// which fails on projects that don't have biome installed.
fn detect_project_linter(root: &Path, ext: &str) -> Option<String> {
    if !matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue") {
        return None;
    }

    // Config files — most reliable signal
    let eslint_configs = [
        ".eslintrc",
        ".eslintrc.json",
        ".eslintrc.js",
        ".eslintrc.cjs",
        ".eslintrc.yaml",
        ".eslintrc.yml",
        "eslint.config.js",
        "eslint.config.mjs",
        "eslint.config.cjs",
        "eslint.config.ts",
    ];
    let biome_configs = ["biome.json", "biome.jsonc"];
    let oxlint_configs = [".oxlintrc.json", "oxlint.config.json"];

    for cfg in eslint_configs {
        if root.join(cfg).exists() {
            return Some("eslint".to_string());
        }
    }
    for cfg in biome_configs {
        if root.join(cfg).exists() {
            return Some("biome".to_string());
        }
    }
    for cfg in oxlint_configs {
        if root.join(cfg).exists() {
            return Some("oxlint".to_string());
        }
    }

    // package.json devDependencies / dependencies
    if let Ok(raw) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(pkg) = serde_json::from_str::<Value>(&raw) {
            let has = |name: &str| {
                ["devDependencies", "dependencies", "peerDependencies"]
                    .iter()
                    .any(|k| {
                        pkg.get(k)
                            .and_then(|v| v.as_object())
                            .map(|o| o.contains_key(name))
                            .unwrap_or(false)
                    })
            };
            if has("eslint") {
                return Some("eslint".to_string());
            }
            if has("@biomejs/biome") {
                return Some("biome".to_string());
            }
            if has("oxlint") {
                return Some("oxlint".to_string());
            }
        }
    }

    None
}

/// If `<root>/node_modules/.bin/<name>` exists, return it. Project-pinned
/// linters often have plugins/configs that the global binary won't load.
fn find_local_node_bin(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let candidate = root.join("node_modules").join(".bin").join(name);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

/// Auto-format file using appropriate formatter
pub async fn tool_format_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let formatter = args.get("formatter").and_then(|v| v.as_str());

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let formatter_name = if let Some(f) = formatter {
        f.to_string()
    } else {
        match ctx.lang_manager.get_language_by_ext(ext) {
            Some(lang_id) => {
                match ctx.lang_manager.get_language(&lang_id) {
                    Some(loaded) => {
                        let f_tool = loaded.manifest.formatter.as_ref().map(|f| f.tool.clone());
                        f_tool.unwrap_or(lang_id)
                    }
                    None => return Err(GoferError::InvalidParams(format!("Language not loaded: {}", lang_id)).into()),
                }
            }
            None => return Err(GoferError::InvalidParams(format!("No language matched for extension: {}", ext)).into()),
        }
    };

    let tool_manifest = match ctx.lang_manager.get_tool(&formatter_name) {
        Some(tm) => tm,
        None => return Err(GoferError::InvalidParams(format!("Tool {} not found in lang-hub", formatter_name)).into()),
    };

    if !tool_manifest.tool.capabilities.contains(&"formatter".to_string()) {
        return Err(GoferError::InvalidParams(format!("Tool {} does not have 'formatter' capability", formatter_name)).into());
    }

    let fmt_config = match &tool_manifest.formatter {
        Some(f) => f.clone(),
        None => return Err(GoferError::InvalidParams(format!("Tool {} has no formatter config", formatter_name)).into()),
    };

    let mut cmd_str = fmt_config.command;
    let mut cmd_args = fmt_config.args;

    if let Some(install) = &tool_manifest.tool.install {
        let exe_name = &install.binary.executable_name;
        let local_exe = ctx.lang_manager.tools_dir.join(&formatter_name).join("bin").join(exe_name);
        if !local_exe.exists() {
            tracing::info!("Tool executable {} not found locally. Attempting to download...", exe_name);
            if let Ok(path) = ctx.lang_manager.download_and_extract_binary(&formatter_name, &tool_manifest).await {
                cmd_str = path.to_string_lossy().to_string();
            }
        } else {
            cmd_str = local_exe.to_string_lossy().to_string();
        }
    }

    cmd_args.push(abs_path.to_string_lossy().to_string());

    // Read original content to detect changes
    let original_content = tokio::fs::read_to_string(&abs_path).await?;
    let original_lines = original_content.lines().count();

    // Run formatter
    let output = Command::new(&cmd_str)
        .args(&cmd_args)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run formatter {}: {}", formatter_name, e))?;

    let result = FormatResult {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    };

    if !result.success {
        tracing::warn!("Formatter {} failed: {}", formatter_name, result.stderr);
    }

    // Read formatted content
    let formatted_content = tokio::fs::read_to_string(&abs_path).await?;
    let formatted_lines = formatted_content.lines().count();

    let changes_made = original_content != formatted_content;
    let diff_lines = (original_lines as i32 - formatted_lines as i32).unsigned_abs();

    // Invalidate cache
    if changes_made {
        ctx.cache.invalidate_file(path).await;
    }

    Ok(json!({
        "path": path,
        "status": if result.success { "formatted" } else { "error" },
        "formatter": formatter_name,
        "changes_made": changes_made,
        "diff_lines": diff_lines,
        "stderr": result.stderr,
    }))
}

/// Run linter on file
pub async fn tool_lint_file(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Allow callers to override detection — useful when a project pins a linter
    // that isn't expressed in package.json (e.g. eslint installed globally).
    let override_linter = args.get("linter").and_then(|v| v.as_str());

    // Detect linter:
    //   1. Explicit override
    //   2. Project-level detection (package.json deps + config files) — wins so
    //      a project using eslint isn't silently linted with biome just because
    //      the typescript manifest defaults to biome.
    //   3. Language manifest default (~/.gofer/langs/<lang>/manifest.toml)
    let linter_name = if let Some(o) = override_linter {
        o.to_string()
    } else if let Some(detected) = detect_project_linter(&ctx.root_path, ext) {
        detected
    } else {
        match ctx.lang_manager.get_language_by_ext(ext) {
            Some(lang_id) => match ctx.lang_manager.get_language(&lang_id) {
                Some(loaded) => loaded
                    .manifest
                    .linter
                    .as_ref()
                    .map(|l| l.tool.clone())
                    .unwrap_or(lang_id),
                None => {
                    return Err(GoferError::InvalidParams(format!(
                        "No linter detected for extension `.{}`. Install eslint/biome/oxlint or pass `linter` explicitly.",
                        ext
                    ))
                    .into());
                }
            },
            None => {
                return Err(GoferError::InvalidParams(format!(
                    "No linter detected for extension `.{}`. Install a linter or pass `linter` explicitly.",
                    ext
                ))
                .into());
            }
        }
    };

    let tool_manifest = match ctx.lang_manager.get_tool(&linter_name) {
        Some(tm) => tm,
        None => return Err(GoferError::InvalidParams(format!("Tool {} not found in lang-hub", linter_name)).into()),
    };

    if !tool_manifest.tool.capabilities.contains(&"linter".to_string()) {
        return Err(GoferError::InvalidParams(format!("Tool {} does not have 'linter' capability", linter_name)).into());
    }

    let lint_config = match &tool_manifest.linter {
        Some(l) => l.clone(),
        None => return Err(GoferError::InvalidParams(format!("Tool {} has no linter config", linter_name)).into()),
    };

    let mut cmd_str = lint_config.command;
    let cmd_args = lint_config.args;

    // Prefer the project-local install in node_modules/.bin when present so we
    // pick up the project's pinned linter version and config plugins.
    if let Some(local_bin) = find_local_node_bin(&ctx.root_path, &linter_name) {
        cmd_str = local_bin.to_string_lossy().to_string();
    } else if let Some(install) = &tool_manifest.tool.install {
        let exe_name = &install.binary.executable_name;
        let local_exe = ctx.lang_manager.tools_dir.join(&linter_name).join("bin").join(exe_name);
        if !local_exe.exists() {
            tracing::info!("Tool executable {} not found locally. Attempting to download...", exe_name);
            if let Ok(path) = ctx.lang_manager.download_and_extract_binary(&linter_name, &tool_manifest).await {
                cmd_str = path.to_string_lossy().to_string();
            }
        } else {
            cmd_str = local_exe.to_string_lossy().to_string();
        }
    }

    // Run linter using appropriate parser
    let result = match linter_name.as_str() {
        "clippy" => lint_with_clippy(&cmd_str, &cmd_args, &abs_path, &ctx.root_path).await?,
        "eslint" => lint_with_eslint(&cmd_str, &cmd_args, &abs_path).await?,
        "ruff" => lint_with_ruff(&cmd_str, &cmd_args, &abs_path).await?,
        "golangci-lint" => lint_with_golangci(&cmd_str, &cmd_args, &abs_path).await?,
        _ => lint_with_generic(&cmd_str, &cmd_args, &abs_path, &linter_name).await?,
    };

    let warnings: Vec<String> = result
        .warnings
        .iter()
        .map(|w| {
            let fix = if w.fix_available {
                " [fix-available]"
            } else {
                ""
            };
            format!(
                "{}:{} [{}] {}: {}{}",
                w.line, w.column, w.severity, w.code, w.message, fix
            )
        })
        .collect();

    Ok(json!({
        "path": path,
        "linter": linter_name,
        "warnings": warnings,
        "errors": result.errors,
        "total_issues": warnings.len() + result.errors.len(),
    }))
}

/// Apply automatic lint fixes
pub async fn tool_apply_lint_fix(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Detect linter
    let linter_name = match ctx.lang_manager.get_language_by_ext(ext) {
        Some(lang_id) => {
            match ctx.lang_manager.get_language(&lang_id) {
                Some(loaded) => {
                    let l_tool = loaded.manifest.linter.as_ref().map(|l| l.tool.clone());
                    l_tool.unwrap_or(lang_id)
                }
                None => return Err(GoferError::InvalidParams(format!("Language not loaded: {}", lang_id)).into()),
            }
        }
        None => return Err(GoferError::InvalidParams(format!("No language matched for extension: {}", ext)).into()),
    };

    // Run auto-fix based on linter
    let result = match linter_name.as_str() {
        "clippy" => apply_clippy_fixes(&abs_path, &ctx.root_path).await?,
        "eslint" => apply_eslint_fixes(&abs_path).await?,
        "ruff" => apply_ruff_fixes(&abs_path).await?,
        _ => {
            return Err(GoferError::InvalidParams(format!(
                "Auto-fix not natively supported in gofer yet for linter: {}",
                linter_name
            ))
            .into())
        }
    };

    // Invalidate cache
    if result.fixes_applied > 0 {
        ctx.cache.invalidate_file(path).await;
    }

    Ok(json!({
        "path": path,
        "status": "fixed",
        "fixes_applied": result.fixes_applied,
        "remaining_warnings": result.remaining_warnings,
    }))
}

// Formatter implementations

#[derive(Debug)]
struct FormatResult {
    success: bool,
    stderr: String,
}



// Linter implementations

#[derive(Debug)]
struct LintResult {
    warnings: Vec<LintWarning>,
    errors: Vec<String>,
}

async fn lint_with_clippy(cmd: &str, args: &[String], path: &Path, project_root: &Path) -> Result<LintResult> {
    let mut command = Command::new(cmd);
    command.args(args);
    command.arg("--message-format=json");
    command.arg("--");
    command.arg("-W");
    command.arg("clippy::all");
    
    let output = command
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run clippy: {}. Is it installed?", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    // Parse clippy JSON output
    for line in stdout.lines() {
        if let Ok(msg) = serde_json::from_str::<Value>(line) {
            if msg.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                if let Some(message) = msg.get("message") {
                    let file_path = message
                        .get("spans")
                        .and_then(|s| s.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|span| span.get("file_name"))
                        .and_then(|f| f.as_str())
                        .unwrap_or("");

                    // Filter only warnings for the specific file
                    if file_path == path.to_string_lossy() {
                        let level = message.get("level").and_then(|l| l.as_str()).unwrap_or("");
                        let msg_text = message
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();
                        let code = message
                            .get("code")
                            .and_then(|c| c.get("code"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();

                        if level == "warning" {
                            warnings.push(LintWarning {
                                line: 0, // Would need to parse from spans
                                column: 0,
                                severity: "warning".to_string(),
                                message: msg_text,
                                code,
                                fix_available: false, // Clippy has --fix but it's project-wide
                            });
                        } else if level == "error" {
                            errors.push(msg_text);
                        }
                    }
                }
            }
        }
    }

    Ok(LintResult { warnings, errors })
}

async fn lint_with_eslint(cmd: &str, args: &[String], path: &Path) -> Result<LintResult> {
    let mut command = Command::new(cmd);
    command.args(args);
    command.arg("--format=json");
    command.arg(path);

    let output = command
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run eslint: {}. Is it installed?", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut warnings = Vec::new();
    let errors = Vec::new();

    // Parse eslint JSON output
    if let Ok(results) = serde_json::from_str::<Value>(&stdout) {
        if let Some(arr) = results.as_array() {
            for result in arr {
                if let Some(messages) = result.get("messages").and_then(|m| m.as_array()) {
                    for msg in messages {
                        warnings.push(LintWarning {
                            line: msg.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                            column: msg.get("column").and_then(|c| c.as_u64()).unwrap_or(0) as u32,
                            severity: msg
                                .get("severity")
                                .and_then(|s| s.as_u64())
                                .map(|s| if s == 2 { "error" } else { "warning" })
                                .unwrap_or("warning")
                                .to_string(),
                            message: msg
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string(),
                            code: msg
                                .get("ruleId")
                                .and_then(|r| r.as_str())
                                .unwrap_or("")
                                .to_string(),
                            fix_available: msg.get("fix").is_some(),
                        });
                    }
                }
            }
        }
    }

    Ok(LintResult { warnings, errors })
}

async fn lint_with_ruff(cmd: &str, args: &[String], path: &Path) -> Result<LintResult> {
    let mut command = Command::new(cmd);
    command.args(args);
    command.arg("--output-format=json");
    command.arg(path);

    let output = command
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run ruff: {}. Is it installed?", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut warnings = Vec::new();
    let errors = Vec::new();

    // Parse ruff JSON output
    if let Ok(results) = serde_json::from_str::<Value>(&stdout) {
        if let Some(arr) = results.as_array() {
            for result in arr {
                warnings.push(LintWarning {
                    line: result
                        .get("location")
                        .and_then(|l| l.get("row"))
                        .and_then(|r| r.as_u64())
                        .unwrap_or(0) as u32,
                    column: result
                        .get("location")
                        .and_then(|l| l.get("column"))
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0) as u32,
                    severity: "warning".to_string(),
                    message: result
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string(),
                    code: result
                        .get("code")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string(),
                    fix_available: result.get("fix").is_some(),
                });
            }
        }
    }

    Ok(LintResult { warnings, errors })
}

async fn lint_with_golangci(_cmd: &str, _args: &[String], _path: &Path) -> Result<LintResult> {
    // golangci-lint is project-level, not file-level
    Ok(LintResult {
        warnings: Vec::new(),
        errors: vec!["golangci-lint requires project-level analysis".to_string()],
    })
}

async fn lint_with_generic(cmd: &str, args: &[String], path: &Path, linter_name: &str) -> Result<LintResult> {
    let mut command = Command::new(cmd);
    command.args(args);
    command.arg(path);

    let output = command
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run linter {}: {}", linter_name, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut warnings = Vec::new();
    let errors = Vec::new();

    // Try to match file:line:col: severity: message
    let re = regex::Regex::new(r"^(?:.+?):(\d+)(?::(\d+))?:\s*(error|warning|info):\s*(.+)$").unwrap();

    let mut found_matches = false;
    for line in stdout.lines() {
        if let Some(caps) = re.captures(line) {
            found_matches = true;
            let line_num = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
            let col_num = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
            let severity = caps.get(3).map_or("warning", |m| m.as_str());
            let message = caps.get(4).map_or("", |m| m.as_str());

            warnings.push(LintWarning {
                line: line_num,
                column: col_num,
                severity: severity.to_string(),
                message: message.to_string(),
                code: "".to_string(),
                fix_available: false,
            });
        }
    }

    if !found_matches && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr).trim().to_string();
        if !combined.is_empty() {
            warnings.push(LintWarning {
                line: 0,
                column: 0,
                severity: "error".to_string(),
                message: combined.chars().take(500).collect(), // truncate if too long
                code: "".to_string(),
                fix_available: false,
            });
        }
    }

    Ok(LintResult { warnings, errors })
}

// Auto-fix implementations

#[derive(Debug)]
struct FixResult {
    fixes_applied: u32,
    remaining_warnings: u32,
}

async fn apply_clippy_fixes(_path: &Path, project_root: &Path) -> Result<FixResult> {
    // Note: cargo clippy --fix is project-wide
    let output = Command::new("cargo")
        .arg("clippy")
        .arg("--fix")
        .arg("--allow-dirty")
        .arg("--allow-staged")
        .current_dir(project_root)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run cargo clippy --fix: {}", e))?;

    // Approximate - we don't have exact counts
    Ok(FixResult {
        fixes_applied: if output.status.success() { 1 } else { 0 },
        remaining_warnings: 0,
    })
}

async fn apply_eslint_fixes(path: &Path) -> Result<FixResult> {
    let output = Command::new("eslint")
        .arg("--fix")
        .arg(path)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run eslint --fix: {}", e))?;

    Ok(FixResult {
        fixes_applied: if output.status.success() { 1 } else { 0 },
        remaining_warnings: 0,
    })
}

async fn apply_ruff_fixes(path: &Path) -> Result<FixResult> {
    let output = Command::new("ruff")
        .arg("check")
        .arg("--fix")
        .arg(path)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run ruff --fix: {}", e))?;

    Ok(FixResult {
        fixes_applied: if output.status.success() { 1 } else { 0 },
        remaining_warnings: 0,
    })
}
