//! Execution Sandbox - Phase 3 implementation
//!
//! Безопасное выполнение кода в изолированном окружении.
//!
//! Implements:
//! - execute_code - выполнить произвольный код
//! - execute_function - выполнить конкретную функцию
//! - run_test - запустить тесты
//! - run_all_tests - запустить все тесты проекта

use super::common::{resolve_path_buf, ToolContext};
use crate::error::GoferError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_SECONDS: u64 = 15;
const MAX_TIMEOUT_SECONDS: u64 = 60;

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: String, // "success", "error", "timeout"
    pub result: Option<Value>,
    pub stdout: String,
    pub stderr: String,
    pub execution_time_ms: u64,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
}

/// Execute arbitrary code (simple wrapper for testing)
pub async fn tool_execute_code(args: Value, ctx: &ToolContext) -> Result<Value> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("code is required".into()))?;

    let language = args
        .get("language")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("language is required".into()))?;

    let timeout_seconds = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .min(MAX_TIMEOUT_SECONDS);

    let lang_id = match ctx.lang_manager.get_language_by_ext(language) {
        Some(id) => id,
        None => language.to_string(),
    };

    let loaded_lang = match ctx.lang_manager.get_language(&lang_id) {
        Some(lang) => lang,
        None => return Err(GoferError::InvalidParams(format!("Unsupported language: {}", language)).into()),
    };

    let sandbox_config = match &loaded_lang.manifest.sandbox {
        Some(sb) => sb.clone(),
        None => return Err(GoferError::InvalidParams(format!("No sandbox config for language: {}", lang_id)).into()),
    };

    let ext = loaded_lang.manifest.language.extensions.first().map(|s| s.as_str()).unwrap_or(lang_id.as_str());

    let temp_dir = tempfile::tempdir()?;
    let src_path = temp_dir.path().join(format!("temp.{}", ext));
    let out_path = temp_dir.path().join("temp_out");

    let full_code = if lang_id == "rust" && !code.contains("fn main") {
        format!("fn main() {{\n{}\n}}", code)
    } else {
        code.to_string()
    };

    tokio::fs::write(&src_path, full_code).await?;

    let start = std::time::Instant::now();

    // Compilation step
    if !sandbox_config.compile_cmd.is_empty() {
        let compile_cmd_str = sandbox_config.compile_cmd
            .replace("{src}", &src_path.to_string_lossy())
            .replace("{out}", &out_path.to_string_lossy());
            
        let mut parts = shell_words::split(&compile_cmd_str).unwrap_or_else(|_| vec![compile_cmd_str.clone()]);
        if !parts.is_empty() {
            let cmd = parts.remove(0);
            let compile_output = Command::new(cmd)
                .args(parts)
                .current_dir(temp_dir.path())
                .output()
                .await?;
                
            if !compile_output.status.success() {
                let stderr_str = String::from_utf8_lossy(&compile_output.stderr).to_string();
                tracing::warn!("Sandbox compilation failed: {}", stderr_str);
                return Ok(serde_json::to_value(ExecutionResult {
                    status: "error".to_string(),
                    result: None,
                    stdout: String::new(),
                    stderr: stderr_str,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error_type: Some("compilation_error".to_string()),
                    error_message: Some("Failed to compile".to_string()),
                })?);
            }
        }
    }

    let confirm_msg = format!("$ tool_execute_code ({})\n\n{}", language, code);
    if !ctx.state.request_confirmation(&confirm_msg).await {
        return Err(GoferError::ToolError("Execution denied by user via TUI".into()).into());
    }

    // Execution step
    let run_cmd_str = sandbox_config.run_cmd
        .replace("{src}", &src_path.to_string_lossy())
        .replace("{out}", &out_path.to_string_lossy());
        
    let mut parts = shell_words::split(&run_cmd_str).unwrap_or_else(|_| vec![run_cmd_str.clone()]);
    if parts.is_empty() {
        return Err(GoferError::InvalidParams("run_cmd is empty".into()).into());
    }
    
    let cmd = parts.remove(0);
    
    let mut command = Command::new(&cmd);
    // If the command is a local file (e.g. "./temp_out"), ensure it exists relative to temp_dir
    if cmd.starts_with("./") {
        command = Command::new(temp_dir.path().join(cmd.trim_start_matches("./")));
    }
    
    let execute_future = command
        .args(parts)
        .current_dir(temp_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let output = match timeout(Duration::from_secs(timeout_seconds), execute_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            tracing::warn!("Sandbox execution error: {}", e);
            return Ok(serde_json::to_value(ExecutionResult {
                status: "error".to_string(),
                result: None,
                stdout: String::new(),
                stderr: e.to_string(),
                execution_time_ms: start.elapsed().as_millis() as u64,
                error_type: Some("execution_error".to_string()),
                error_message: Some(e.to_string()),
            })?)
        }
        Err(_) => {
            tracing::warn!("Sandbox execution timed out after {} seconds", timeout_seconds);
            return Ok(serde_json::to_value(ExecutionResult {
                status: "timeout".to_string(),
                result: None,
                stdout: String::new(),
                stderr: format!("Execution timed out after {} seconds", timeout_seconds),
                execution_time_ms: start.elapsed().as_millis() as u64,
                error_type: Some("timeout".to_string()),
                error_message: Some("Timeout exceeded".to_string()),
            })?)
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let result = ExecutionResult {
        status: if output.status.success() { "success" } else { "error" }.to_string(),
        result: if output.status.success() { Some(json!(stdout.trim())) } else { None },
        stdout,
        stderr: stderr.clone(),
        execution_time_ms: start.elapsed().as_millis() as u64,
        error_type: if output.status.success() { None } else { Some("runtime_error".to_string()) },
        error_message: if output.status.success() { None } else { Some(stderr) },
    };

    Ok(serde_json::to_value(&result)?)
}

/// Execute specific function from file
pub async fn tool_execute_function(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let function_name = args
        .get("function_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("function_name is required".into()))?;

    let function_args = args
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let timeout_seconds = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .min(MAX_TIMEOUT_SECONDS);

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let result = match ext {
        "rs" => {
            json!({
                "status": "error",
                "stderr": "execute_function for Rust requires cargo integration. Use run_test instead.",
                "error_type": "not_implemented"
            })
        }
        "py" => {
            let args_json = serde_json::to_string(&function_args)?;
            let code = format!(
                r#"
import sys
import json
sys.path.insert(0, '{}')
from {} import {}

args = json.loads('{}')
result = {}(*args)
print(json.dumps(result))
"#,
                abs_path.parent().unwrap_or(Path::new("")).display(),
                abs_path.file_stem().unwrap_or_default().to_string_lossy(),
                function_name,
                args_json.replace('\'', "\\'"),
                function_name
            );
            return tool_execute_code(json!({ "code": code, "language": "python", "timeout": timeout_seconds }), ctx).await;
        }
        "js" | "ts" => {
            let args_json = serde_json::to_string(&function_args)?;
            let code = format!(
                r#"
const module = require('{}');
const args = {};
const result = module.{}(...args);
console.log(JSON.stringify(result));
"#,
                abs_path.display(),
                args_json,
                function_name
            );
            return tool_execute_code(json!({ "code": code, "language": "javascript", "timeout": timeout_seconds }), ctx).await;
        }
        _ => {
            return Err(
                GoferError::InvalidParams(format!("Unsupported file extension: {}", ext)).into(),
            )
        }
    };

    Ok(result)
}

/// Run specific test
#[allow(dead_code)]
pub async fn tool_run_test(args: Value, ctx: &ToolContext) -> Result<Value> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GoferError::InvalidParams("path is required".into()))?;

    let test_name = args.get("test_name").and_then(|v| v.as_str());

    let timeout_seconds = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .min(MAX_TIMEOUT_SECONDS);

    let abs_path = resolve_path_buf(&ctx.root_path, path)?;

    if !abs_path.exists() {
        return Err(GoferError::InvalidParams(format!("File not found: {}", path)).into());
    }

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let result = match ext {
        "rs" => run_rust_test(&ctx.root_path, test_name, timeout_seconds).await?,
        "py" => run_python_test(&abs_path, test_name, timeout_seconds).await?,
        "js" | "ts" => run_javascript_test(&abs_path, test_name, timeout_seconds).await?,
        _ => {
            return Err(
                GoferError::InvalidParams(format!("Unsupported file extension: {}", ext)).into(),
            )
        }
    };

    Ok(result)
}

/// Run all tests in project
#[allow(dead_code)]
pub async fn tool_run_all_tests(args: Value, ctx: &ToolContext) -> Result<Value> {
    let filter = args.get("filter").and_then(|v| v.as_str());

    let timeout_seconds = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .min(MAX_TIMEOUT_SECONDS * 2); // Allow longer for all tests

    let project_root = &ctx.root_path;

    for entry in ctx.lang_manager.loaded_langs.iter() {
        let manifest = &entry.value().manifest;
        if manifest.language.root_markers.iter().any(|marker| project_root.join(marker).exists()) {
            if let Some(sandbox) = &manifest.sandbox {
                if let Some(test_cmd_parts) = &sandbox.test {
                    let confirm_msg = format!("$ tool_run_all_tests\n\nCommand: {}", test_cmd_parts.join(" "));
                    if !ctx.state.request_confirmation(&confirm_msg).await {
                        return Err(GoferError::ToolError("Execution denied by user via TUI".into()).into());
                    }
                    return execute_generic_tests(project_root, filter, timeout_seconds, test_cmd_parts).await;
                }
            }
        }
    }

    Err(GoferError::InvalidParams("No test framework detected in project via lang-hub manifests".into()).into())
}





// Test runner implementations

#[allow(dead_code)]
async fn run_rust_test(
    project_root: &Path,
    test_name: Option<&str>,
    timeout_secs: u64,
) -> Result<Value> {
    let start = std::time::Instant::now();

    let mut cmd = Command::new("cargo");
    cmd.arg("test")
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(name) = test_name {
        cmd.arg(name);
    }

    cmd.arg("--");
    cmd.arg("--nocapture");

    let execute_future = cmd.output();

    let output = match timeout(Duration::from_secs(timeout_secs), execute_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Ok(json!({
                "status": "error",
                "error": e.to_string(),
            }))
        }
        Err(_) => {
            return Ok(json!({
                "status": "timeout",
                "message": format!("Tests timed out after {} seconds", timeout_secs),
            }))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse test results (simplified)
    let status = if output.status.success() {
        "passed"
    } else {
        "failed"
    };

    Ok(json!({
        "status": status,
        "execution_time_ms": start.elapsed().as_millis(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

#[allow(dead_code)]
async fn run_python_test(path: &Path, test_name: Option<&str>, timeout_secs: u64) -> Result<Value> {
    let start = std::time::Instant::now();

    let mut cmd = Command::new("pytest");
    cmd.arg(path)
        .arg("-v")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(name) = test_name {
        cmd.arg("-k").arg(name);
    }

    let execute_future = cmd.output();

    let output = match timeout(Duration::from_secs(timeout_secs), execute_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Ok(json!({
                "status": "error",
                "error": format!("pytest not found or execution error: {}", e),
            }))
        }
        Err(_) => {
            return Ok(json!({
                "status": "timeout",
                "message": format!("Tests timed out after {} seconds", timeout_secs),
            }))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    let status = if output.status.success() {
        "passed"
    } else {
        "failed"
    };

    Ok(json!({
        "status": status,
        "execution_time_ms": start.elapsed().as_millis(),
        "output": stdout,
    }))
}

#[allow(dead_code)]
async fn run_javascript_test(
    path: &Path,
    test_name: Option<&str>,
    timeout_secs: u64,
) -> Result<Value> {
    let start = std::time::Instant::now();

    // Try Jest first
    let mut cmd = Command::new("npx");
    cmd.arg("jest")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(name) = test_name {
        cmd.arg("-t").arg(name);
    }

    let execute_future = cmd.output();

    let output = match timeout(Duration::from_secs(timeout_secs), execute_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Ok(json!({
                "status": "error",
                "error": format!("jest not found: {}", e),
            }))
        }
        Err(_) => {
            return Ok(json!({
                "status": "timeout",
                "message": format!("Tests timed out after {} seconds", timeout_secs),
            }))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    let status = if output.status.success() {
        "passed"
    } else {
        "failed"
    };

    Ok(json!({
        "status": status,
        "execution_time_ms": start.elapsed().as_millis(),
        "output": stdout,
    }))
}

async fn execute_generic_tests(
    project_root: &Path,
    filter: Option<&str>,
    timeout_secs: u64,
    test_cmd_parts: &[String],
) -> Result<Value> {
    if test_cmd_parts.is_empty() {
        return Err(GoferError::InvalidParams("Test command is empty in lang-hub manifest".into()).into());
    }

    let mut cmd = Command::new(&test_cmd_parts[0]);
    if test_cmd_parts.len() > 1 {
        cmd.args(&test_cmd_parts[1..]);
    }
    cmd.current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(f) = filter {
        cmd.arg(f);
    }

    let start = std::time::Instant::now();
    let execute_future = cmd.output();

    let output = match timeout(Duration::from_secs(timeout_secs), execute_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Ok(json!({
                "status": "error",
                "error": e.to_string(),
            }))
        }
        Err(_) => {
            return Ok(json!({
                "status": "timeout",
                "message": format!("Tests timed out after {} seconds", timeout_secs),
            }))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Ok(json!({
        "status": if output.status.success() { "passed" } else { "failed" },
        "execution_time_ms": start.elapsed().as_millis(),
        "stdout": stdout.to_string(),
        "stderr": stderr.to_string(),
    }))
}
