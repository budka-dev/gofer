use super::common::ToolContext;
use super::files::{
    tool_read_function_context, tool_read_types_only, tool_skeleton,
};
use super::search::tool_search;
use super::symbols::{tool_get_references, tool_get_symbols};
use crate::error::GoferError;
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn tool_batch_operations(args: Value, ctx: &ToolContext) -> Result<Value> {
    use std::time::Instant;
    use tokio::sync::Semaphore;

    // Config
    const MAX_BATCH_SIZE: usize = 100;
    const MAX_CONCURRENT: usize = 10;

    let operations = args
        .get("operations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| GoferError::InvalidParams("operations array is required".into()))?;

    // Validate batch size
    if operations.len() > MAX_BATCH_SIZE {
        return Err(GoferError::InvalidParams(format!(
            "Too many operations. Max: {}, got: {}",
            MAX_BATCH_SIZE,
            operations.len()
        ))
        .into());
    }

    let parallel = args
        .get("parallel")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // NOW default to true!

    let continue_on_error = args
        .get("continue_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Output trimming controls — let callers cap the response size when they
    // only need success/timing info or a preview. The previous behaviour (full
    // payload of every sub-operation) frequently produced 50KB+ responses for
    // 3-call batches, which is why callers preferred firing three separate
    // tool calls.
    let summary_only = args
        .get("summary_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_chars_per_op = args
        .get("max_chars_per_op")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let start = Instant::now();
    let mut results;

    if parallel {
        // Parallel execution with rate limiting
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));

        let tasks: Vec<_> = operations
            .iter()
            .enumerate()
            .map(|(idx, operation)| {
                let ctx = ctx.clone(); // NOW we can clone!
                let op = operation.clone();
                let sem = Arc::clone(&semaphore);

                tokio::spawn(async move {
                    let _permit = match sem.acquire().await {
                        Ok(p) => p,
                        Err(e) => return Err(anyhow::anyhow!("Semaphore closed: {}", e)),
                    };
                    execute_single_operation(idx, op, &ctx).await
                })
            })
            .collect();

        // Await all tasks
        let mut batch_results = Vec::new();
        for task in tasks {
            match task.await {
                Ok(Ok(result)) => batch_results.push(result),
                Ok(Err(e)) => {
                    if !continue_on_error {
                        return Err(e);
                    }
                    batch_results.push(create_error_result(e));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Task join error: {}", e));
                }
            }
        }

        results = batch_results;
    } else {
        // Sequential execution (fallback)
        results = execute_sequential(operations, ctx, continue_on_error).await?;
    }

    let total_duration_ms = start.elapsed().as_millis() as u64;

    let successful = results
        .iter()
        .filter(|r| r["success"].as_bool().unwrap_or(false))
        .count();
    let failed = results.len() - successful;

    // Apply requested trimming. We do this after execution so we still get
    // accurate timings from the original Value before mutating it.
    if summary_only {
        for r in results.iter_mut() {
            if let Some(obj) = r.as_object_mut() {
                obj.remove("data");
            }
        }
    } else if let Some(max) = max_chars_per_op {
        for r in results.iter_mut() {
            if let Some(obj) = r.as_object_mut() {
                if let Some(data) = obj.get("data").cloned() {
                    let serialized = data.to_string();
                    if serialized.len() > max {
                        let mut truncated: String = serialized.chars().take(max).collect();
                        truncated.push_str("…[truncated]");
                        obj.insert("data".to_string(), json!(truncated));
                        obj.insert("data_truncated".to_string(), json!(true));
                        obj.insert(
                            "data_full_chars".to_string(),
                            json!(serialized.len()),
                        );
                    }
                }
            }
        }
    }

    Ok(json!({
        "total_operations": operations.len(),
        "successful": successful,
        "failed": failed,
        "parallel": parallel,
        "summary_only": summary_only,
        "total_duration_ms": total_duration_ms,
        "results": results
    }))
}

/// Execute a single operation (helper for batch_operations)
async fn execute_single_operation(
    idx: usize,
    operation: Value,
    ctx: &ToolContext,
) -> Result<Value> {
    let op_type = operation
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let params = operation.get("params").cloned().unwrap_or(json!({}));

    let op_start = std::time::Instant::now();

    let (success, data, error) = match op_type {
        "get_symbols" => match tool_get_symbols(params, ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        },
        "search" => match tool_search(params, ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        },
        "skeleton" => match tool_skeleton(params, ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        },
        "get_references" => match tool_get_references(params, ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        },
        "read_function_context" => match tool_read_function_context(params, ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        },
        "read_types_only" => match tool_read_types_only(params, ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        },
        _ => (
            false,
            None,
            Some(format!("Unknown operation type: {}", op_type)),
        ),
    };

    let duration_ms = op_start.elapsed().as_millis() as u64;

    Ok(json!({
        "index": idx,
        "type": op_type,
        "success": success,
        "data": data,
        "error": error,
        "duration_ms": duration_ms
    }))
}

/// Sequential execution fallback
async fn execute_sequential(
    operations: &[Value],
    ctx: &ToolContext,
    continue_on_error: bool,
) -> Result<Vec<Value>> {
    let mut results = Vec::new();

    for (idx, operation) in operations.iter().enumerate() {
        let result = execute_single_operation(idx, operation.clone(), ctx).await?;

        let success = result["success"].as_bool().unwrap_or(false);
        results.push(result);

        if !success && !continue_on_error {
            break;
        }
    }

    Ok(results)
}

/// Create error result for failed operations
fn create_error_result(error: anyhow::Error) -> Value {
    json!({
        "success": false,
        "error": error.to_string(),
        "duration_ms": 0
    })
}
