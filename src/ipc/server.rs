//! Unix socket daemon server — accepts connections and routes JSON-RPC requests.

use std::sync::Arc;

use crate::error::GoferError;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixListener;

use super::protocol::{DaemonNotification, DaemonRequest, DaemonResponse};
use crate::daemon::state::DaemonState;
use crate::daemon::tools;

/// Run the daemon server — blocks until shutdown is requested.
pub async fn run_daemon(state: Arc<DaemonState>) -> Result<()> {
    let socket_path = state.gofer_home.join("daemon.sock");

    // Clean up stale socket
    if socket_path.exists() {
        tokio::fs::remove_file(&socket_path).await?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("Daemon listening on {:?}", socket_path);

    let token = state.shutdown_token.clone();

    // Accept connections until shutdown token is cancelled
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _addr) = result?;
                let state = state.clone();

                // Acquire connection permit (bounded concurrency)
                let permit = match state.connection_semaphore.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!(
                            "Max connections reached (1024), rejecting new connection (current: {})",
                            1024 - state.connection_semaphore.available_permits()
                        );
                        continue;
                    }
                };

                tokio::spawn(async move {
                    let conn_start = std::time::Instant::now();
                    tracing::info!("New connection accepted (active connections: {})", 1024 - state.connection_semaphore.available_permits());

                    if let Err(e) = handle_connection(stream, state.clone()).await {
                        tracing::error!("Connection error: {}", e);
                    }

                    let duration = conn_start.elapsed();
                    tracing::info!(
                        "Connection closed after {:?} (active connections: {})",
                        duration,
                        1024 - state.connection_semaphore.available_permits() - 1
                    );
                    drop(permit); // release on exit
                });
            }
            _ = token.cancelled() => {
                tracing::info!("Shutdown token triggered, stopping accept loop");
                break;
            }
        }
    }

    // Clean up socket file
    let _ = tokio::fs::remove_file(&socket_path).await;
    tracing::info!("Daemon stopped gracefully");
    Ok(())
}

async fn handle_connection(stream: tokio::net::UnixStream, state: Arc<DaemonState>) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let writer = BufWriter::new(writer);
    let mut line = String::new();

    // Channel for writing responses and notifications to the connection
    let (write_tx, mut write_rx) = tokio::sync::mpsc::channel::<String>(64);

    // Subscribe to broadcast notifications (e.g. tools/list_changed)
    let mut notify_rx = state.notify_tx.subscribe();
    let notify_write_tx = write_tx.clone();
    tokio::spawn(async move {
        while let Ok(msg) = notify_rx.recv().await {
            if notify_write_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Writer task: drains channel and writes to the socket
    let writer_handle = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(msg) = write_rx.recv().await {
            if let Err(e) = async {
                writer.write_all(msg.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                Ok::<(), std::io::Error>(())
            }
            .await
            {
                tracing::error!("Write error: {}", e);
                break;
            }
        }
    });

    // Per-connection rate limiter: max 1000 requests per second
    const RATE_LIMIT: u32 = 1000;
    // Idle timeout: close connection after 6 hours of inactivity
    const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(21600);

    let mut window_start = std::time::Instant::now();
    let mut window_count: u32 = 0;

    loop {
        line.clear();

        // Read with timeout to detect idle connections
        let read_result = tokio::time::timeout(IDLE_TIMEOUT, reader.read_line(&mut line)).await;

        let n = match read_result {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                tracing::info!("Connection idle timeout after {:?}, closing", IDLE_TIMEOUT);
                break;
            }
        };

        if n == 0 {
            break; // EOF — client disconnected
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Rate limiting: sliding window (1 second)
        let now = std::time::Instant::now();
        if now.duration_since(window_start).as_secs() >= 1 {
            window_start = now;
            window_count = 0;
        }
        window_count += 1;

        if window_count > RATE_LIMIT {
            let resp = DaemonResponse::error(
                Value::Null,
                -32000,
                "Rate limit exceeded (1000 req/s per connection)".into(),
            );
            let out =
                simd_json::to_string(&resp).map_err(|e| anyhow::anyhow!("simd_json: {}", e))?;
            let _ = write_tx.send(out).await;
            continue;
        }

        // Try to parse as batch (JSON array) first, then single request
        let mut parse_bytes = trimmed.as_bytes().to_vec();
        let parsed: Result<Value, _> =
            simd_json::from_slice(&mut parse_bytes).map_err(|e| anyhow::anyhow!("{}", e));

        match parsed {
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                // JSON-RPC 2.0 Batch Request — process in parallel
                let state_clone = state.clone();
                let handles: Vec<_> = batch
                    .into_iter()
                    .map(|v| {
                        let st = state_clone.clone();
                        tokio::spawn(async move {
                            match serde_json::from_value::<DaemonRequest>(v) {
                                Ok(req) => {
                                    let is_notification = req.id.is_none();
                                    let resp = handle_request(req, &st).await;
                                    if is_notification {
                                        None
                                    } else {
                                        Some(resp)
                                    }
                                }
                                Err(e) => {
                                    let (code, msg) =
                                        GoferError::ParseError(e.to_string()).into_rpc();
                                    Some(DaemonResponse::error(Value::Null, code, msg))
                                }
                            }
                        })
                    })
                    .collect();

                // Await all in parallel and collect results
                let mut responses = Vec::with_capacity(handles.len());
                for handle in handles {
                    if let Ok(Some(resp)) = handle.await {
                        responses.push(resp);
                    }
                }

                // Send batch response as JSON array
                if !responses.is_empty() {
                    let out = simd_json::to_string(&responses)
                        .map_err(|e| anyhow::anyhow!("simd_json: {}", e))?;
                    let _ = write_tx.send(out).await;
                }
            }
            _ => {
                // Single request (original logic)
                let mut single_bytes = trimmed.as_bytes().to_vec();
                let response = match simd_json::from_slice::<DaemonRequest>(&mut single_bytes) {
                    Ok(req) => {
                        let is_notification = req.id.is_none();

                        // Check for progress token in _meta
                        let progress_token = req
                            .params
                            .get("_meta")
                            .and_then(|m| m.get("progressToken"))
                            .and_then(|t| t.as_str())
                            .map(String::from);

                        // If this is a reindex request with a progress token, spawn progress reporter
                        let _progress_guard = if let Some(token) = progress_token {
                            if req.method == "reindex" || req.method == "daemon/activate_project" {
                                let notif_tx = write_tx.clone();
                                let progress = state.sync_progress.clone();
                                let cancel = tokio_util::sync::CancellationToken::new();
                                let cancel_clone = cancel.clone();
                                tokio::spawn(async move {
                                    let mut interval = tokio::time::interval(
                                        std::time::Duration::from_millis(500),
                                    );
                                    loop {
                                        tokio::select! {
                                            _ = interval.tick() => {}
                                            _ = cancel_clone.cancelled() => { break; }
                                        }
                                        let snap = progress.snapshot();
                                        if !snap.active {
                                            continue;
                                        }
                                        let stage = progress.stage.lock().await.clone();
                                        let notif = DaemonNotification::progress(
                                            &token,
                                            snap.files_parsed,
                                            snap.files_total,
                                            &stage,
                                        );
                                        if let Ok(msg) = simd_json::to_string(&notif) {
                                            if notif_tx.send(msg).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                });
                                Some(cancel)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if req.method != "ping" {
                            tracing::info!("Incoming request: {}", req.method);
                        }
                        let resp = handle_request(req, &state).await;

                        // Stop progress reporter
                        if let Some(cancel) = _progress_guard {
                            cancel.cancel();
                        }

                        if is_notification {
                            None
                        } else {
                            Some(resp)
                        }
                    }
                    Err(e) => {
                        let (code, msg) = GoferError::ParseError(e.to_string()).into_rpc();
                        Some(DaemonResponse::error(Value::Null, code, msg))
                    }
                };

                if let Some(resp) = response {
                    let out = simd_json::to_string(&resp)
                        .map_err(|e| anyhow::anyhow!("simd_json: {}", e))?;
                    let _ = write_tx.send(out).await;
                }
            }
        }
    }

    drop(write_tx);
    let _ = writer_handle.await;

    Ok(())
}

async fn handle_request(req: DaemonRequest, state: &Arc<DaemonState>) -> DaemonResponse {
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        // === Daemon management ===
        "daemon/register_project" => handle_register_project(id, &req.params, state).await,
        "daemon/activate_project" => handle_activate_project(id, &req.params, state).await,
        "daemon/deactivate_project" => handle_deactivate_project(id, &req.params, state).await,
        "daemon/sync_progress" => handle_sync_progress(id, state).await,
        "daemon/status" => handle_status(id, state).await,
        "daemon/health" => handle_health(id, state).await,
        "daemon/summary_stats" => handle_summary_stats(id, &req.params, state).await,
        "daemon/metrics" => handle_metrics(id, state).await,
        "daemon/shutdown" => handle_shutdown(id, state).await,
        "reindex" => handle_reindex(id, &req.params, state).await,
        // === MCP protocol methods ===
        "initialize" => DaemonResponse::success(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "gofer",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": { "listChanged": true },
                    "resources": { "subscribe": false, "listChanged": false },
                    "prompts": { "listChanged": false }
                }
            }),
        ),
        "initialized" | "notifications/initialized" => DaemonResponse::success(id, json!({})),
        "ping" => DaemonResponse::success(id, json!({})),
        "tools/list" => handle_tools_list(id, &req, state).await,
        "tools/call" => handle_tools_call(id, &req, state).await,
        "resources/list" => handle_resources_list(id, &req, state).await,
        "resources/read" => handle_resources_read(id, &req, state).await,
        "prompts/list" => handle_prompts_list(id).await,
        "prompts/get" => handle_prompts_get(id, &req, state).await,

        _ => {
            let (code, msg) = GoferError::MethodNotFound(req.method.clone()).into_rpc();
            DaemonResponse::error(id, code, msg)
        }
    }
}

// === Daemon management handlers ===

async fn handle_register_project(
    id: Value,
    params: &Value,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let project_path = match params.get("project_path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return DaemonResponse::error(id, -32602, "Missing project_path".into()),
    };

    match state.registry.register(project_path).await {
        Ok(uuid) => {
            // Create index directory
            let index_dir = state.gofer_home.join("indices").join(&uuid);
            let _ = tokio::fs::create_dir_all(&index_dir).await;

            tracing::info!("Registered project: {} -> {}", project_path, uuid);
            DaemonResponse::success(id, json!({ "id": uuid, "path": project_path }))
        }
        Err(e) => DaemonResponse::error(id, -32000, format!("Registration failed: {}", e)),
    }
}

async fn handle_activate_project(
    id: Value,
    params: &Value,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let project_path = match params.get("project_path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return DaemonResponse::error(id, -32602, "Missing project_path".into()),
    };
    let watch = params
        .get("watch")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let background = params
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match state
        .activate_project(project_path, watch, background)
        .await
    {
        Ok(msg) => DaemonResponse::success(id, json!({ "message": msg })),
        Err(e) => DaemonResponse::error(id, -32000, format!("Activation failed: {}", e)),
    }
}

async fn handle_deactivate_project(
    id: Value,
    params: &Value,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let project_path = match params.get("project_path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return DaemonResponse::error(id, -32602, "Missing project_path".into()),
    };

    match state.deactivate_project(project_path).await {
        Ok(()) => DaemonResponse::success(id, json!({ "message": "Deactivated" })),
        Err(e) => DaemonResponse::error(id, -32000, format!("Deactivation failed: {}", e)),
    }
}

async fn handle_sync_progress(id: Value, state: &Arc<DaemonState>) -> DaemonResponse {
    let snap = state.sync_progress.snapshot();
    let stage = state.sync_progress.stage.lock().await.clone();
    DaemonResponse::success(
        id,
        json!({
            "active": snap.active,
            "stage": stage,
            "files_total": snap.files_total,
            "files_scanned": snap.files_scanned,
            "files_parsed": snap.files_parsed,
            "chunks_embedded": snap.chunks_embedded,
            "files_written": snap.files_written,
        }),
    )
}

async fn handle_status(id: Value, state: &Arc<DaemonState>) -> DaemonResponse {
    let uptime = state.started_at.elapsed().as_secs();
    let projects = state.projects.read().await;
    let project_list: Vec<Value> = projects
        .values()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "path": p.path.to_string_lossy(),
            })
        })
        .collect();

    let all_projects = state.registry.list().await.unwrap_or_default();

    DaemonResponse::success(
        id,
        json!({
            "uptime_seconds": uptime,
            "active_projects": project_list,
            "registered_projects": all_projects.len(),
        }),
    )
}

async fn handle_health(id: Value, state: &Arc<DaemonState>) -> DaemonResponse {
    let uptime = state.started_at.elapsed().as_secs();
    let projects = state.projects.read().await;
    let active_count = projects.len();

    // Check SQLite connectivity for each active project
    let mut sqlite_ok = true;
    for project in projects.values() {
        if project.sqlite.health_check().await.is_err() {
            sqlite_ok = false;
            break;
        }
    }

    let status = if sqlite_ok { "healthy" } else { "degraded" };

    DaemonResponse::success(
        id,
        json!({
            "status": status,
            "uptime_seconds": uptime,
            "active_projects": active_count,
            "sqlite": sqlite_ok,
            "pid": std::process::id(),
        }),
    )
}

async fn handle_summary_stats(
    id: Value,
    params: &Value,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let project_path = match params.get("project_path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return DaemonResponse::error(id, -32602, "Missing project_path".into()),
    };

    match state.get_or_load_project(project_path).await {
        Ok(_) => DaemonResponse::success(
            id,
            json!({
                "pending": 0,
                "processing": 0,
                "failed": 0,
            }),
        ),
        Err(e) => DaemonResponse::error(id, -32000, format!("Project load failed: {}", e)),
    }
}

async fn handle_metrics(id: Value, state: &Arc<DaemonState>) -> DaemonResponse {
    DaemonResponse::success(id, state.metrics.snapshot())
}

async fn handle_shutdown(id: Value, state: &Arc<DaemonState>) -> DaemonResponse {
    tracing::info!("Shutdown requested via daemon/shutdown");

    // Trigger graceful shutdown via cancellation token
    state.shutdown_token.cancel();

    DaemonResponse::success(id, json!({ "message": "Shutting down" }))
}

async fn handle_reindex(id: Value, params: &Value, state: &Arc<DaemonState>) -> DaemonResponse {
    let project_path = match params.get("project_path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return DaemonResponse::error(id, -32602, "Missing project_path".into()),
    };
    let force = params
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let path = params.get("path").and_then(|v| v.as_str());

    if force {
        // Очистить индекс проекта и пересинхронизировать
        if let Ok(project) = state.get_or_load_project(project_path).await {
            // Очистить SQLite данные (файлы, символы, ссылки)
            let pool = project.sqlite.pool();
            let _ = sqlx::query("DELETE FROM symbol_references")
                .execute(pool)
                .await;
            let _ = sqlx::query("DELETE FROM symbols").execute(pool).await;
            let _ = sqlx::query("DELETE FROM dependency_usage")
                .execute(pool)
                .await;
            let _ = sqlx::query("DELETE FROM files").execute(pool).await;
            tracing::info!("Force reindex: cleared SQLite data for {}", project_path);
        }
        DaemonResponse::success(id, json!({ "message": "Data cleared, ready for resync" }))
    } else if let Some(file_path) = path {
        // Переиндексировать один файл
        match state.get_or_load_project(project_path).await {
            Ok(project) => {
                let indexer = crate::indexer::service::IndexerService::new(
                    project.sqlite.clone(),
                    project.lance.clone(),
                    project.embedder.clone(),
                    1,
                );
                match indexer.index_file(std::path::Path::new(file_path)).await {
                    Ok(_) => DaemonResponse::success(
                        id,
                        json!({ "message": format!("Reindexed: {}", file_path) }),
                    ),
                    Err(e) => DaemonResponse::error(id, -32000, format!("Reindex failed: {}", e)),
                }
            }
            Err(e) => DaemonResponse::error(id, -32000, format!("Project load failed: {}", e)),
        }
    } else {
        DaemonResponse::error(id, -32602, "Specify --force or --path".into())
    }
}

// === MCP tool routing ===

async fn handle_tools_list(
    id: Value,
    _req: &DaemonRequest,
    _state: &Arc<DaemonState>,
) -> DaemonResponse {
    let tool_list = tools::core_tools_list();
    DaemonResponse::success(id, json!({ "tools": tool_list }))
}

async fn handle_tools_call(
    id: Value,
    req: &DaemonRequest,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let name = req
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
    let project_path = req.project_path();

    let Some(pp) = project_path else {
        return DaemonResponse::error(id, -32602, "Missing project_path for tool call".into());
    };

    // Feature 015: Check resource limits before processing request
    let _request_guard = match state.resource_limits.try_acquire_request() {
        Ok(guard) => guard,
        Err(e) => {
            return DaemonResponse::error(id, -32001, format!("Resource limit exceeded: {}", e))
        }
    };

    let project = match state.get_or_load_project(pp).await {
        Ok(p) => p,
        Err(e) => return DaemonResponse::error(id, -32000, format!("Project load failed: {}", e)),
    };

    let ctx = tools::ToolContext {
        sqlite: Arc::new(project.sqlite.clone()),
        lance: Arc::clone(&project.lance),
        embedder: Arc::clone(&project.embedder),
        root_path: Arc::new(project.path.clone()),
        cache: Arc::clone(&project.cache),
        embedding_circuit: Arc::clone(&state.embedding_circuit),
        vector_circuit: Arc::clone(&state.vector_circuit),
    };

    // Core tools
    let start = std::time::Instant::now();

    let result: anyhow::Result<serde_json::Value> = tools::dispatch(name, args.clone(), &ctx).await;
    let elapsed_us = start.elapsed().as_micros() as usize;
    state.metrics.record_query(elapsed_us);

    // M5: Audit log — fire-and-forget, don't block response
    let latency_ms = (elapsed_us / 1000) as u64;
    let success = result.is_ok();
    let err_msg = result.as_ref().err().map(|e: &anyhow::Error| e.to_string());
    let args_str = serde_json::to_string(&args).ok();
    let sqlite_clone = project.sqlite.clone();
    let tool_name = name.to_string();
    tokio::spawn(async move {
        let _ = sqlite_clone
            .log_tool_call(
                &tool_name,
                args_str.as_deref(),
                latency_ms,
                success,
                err_msg.as_deref(),
            )
            .await;
    });

    match result {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            DaemonResponse::success(id, json!({ "content": [{"type": "text", "text": text}] }))
        }
        Err(e) => {
            tracing::error!("Tool Error [{}]: {}", name, e);
            DaemonResponse::success(
                id,
                json!({
                    "content": [{"type": "text", "text": format!("Error: {}", e)}],
                    "isError": true
                }),
            )
        }
    }
}

// === MCP Resources ===

async fn handle_resources_list(
    id: Value,
    req: &DaemonRequest,
    _state: &Arc<DaemonState>,
) -> DaemonResponse {
    let _ = req;
    DaemonResponse::success(
        id,
        json!({
            "resources": [
                {
                    "uri": "project://context",
                    "name": "Project Context",
                    "description": "Full project context: rules, golden samples, dependencies",
                    "mimeType": "application/json"
                },
                {
                    "uri": "project://tree",
                    "name": "Project Tree",
                    "description": "Directory structure of the project",
                    "mimeType": "application/json"
                },
                {
                    "uri": "project://stats",
                    "name": "Project Stats",
                    "description": "Code statistics: file count, symbols, domain distribution",
                    "mimeType": "application/json"
                },
                {
                    "uri": "project://config",
                    "name": "Project Config",
                    "description": "Discovered configuration keys from .env.example files",
                    "mimeType": "application/json"
                }
            ]
        }),
    )
}

async fn handle_resources_read(
    id: Value,
    req: &DaemonRequest,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let uri = req.params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
    let project_path = req.project_path();

    let Some(pp) = project_path else {
        return DaemonResponse::error(id, -32602, "Missing project_path".into());
    };

    let project = match state.get_or_load_project(pp).await {
        Ok(p) => p,
        Err(e) => return DaemonResponse::error(id, -32000, format!("Project load failed: {}", e)),
    };

    let ctx = tools::ToolContext {
        sqlite: Arc::new(project.sqlite.clone()),
        lance: Arc::clone(&project.lance),
        embedder: Arc::clone(&project.embedder),
        root_path: Arc::new(project.path.clone()),
        cache: Arc::clone(&project.cache),
        embedding_circuit: Arc::clone(&state.embedding_circuit),
        vector_circuit: Arc::clone(&state.vector_circuit),
    };

    let result = match uri {
        "project://context" => resource_project_context(&ctx).await,
        "project://tree" => tools::dispatch("project_tree", json!({"depth": 3}), &ctx).await,
        "project://stats" => resource_project_stats(&ctx).await,
        "project://config" => tools::dispatch("get_config_keys", json!({}), &ctx).await,
        _ => {
            return DaemonResponse::error(id, -32602, format!("Unknown resource URI: {}", uri));
        }
    };

    match result {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            DaemonResponse::success(
                id,
                json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": text
                    }]
                }),
            )
        }
        Err(e) => DaemonResponse::error(id, -32000, format!("Resource read error: {}", e)),
    }
}

async fn resource_project_context(ctx: &tools::ToolContext) -> anyhow::Result<Value> {
    use crate::models::chunk::Rule;

    let rules: Vec<Rule> = ctx.sqlite.get_rules().await?;
    let golden_samples: Vec<(String, Option<String>)> = ctx.sqlite.get_golden_samples().await?;
    let deps: Vec<crate::models::chunk::Dependency> =
        ctx.sqlite.get_dependencies_filtered(None).await?;

    Ok(json!({
        "rules": rules.iter().map(|r| json!({
            "category": r.category,
            "rule": r.rule,
            "priority": r.priority,
            "source": r.source
        })).collect::<Vec<_>>(),
        "golden_samples": golden_samples.iter().map(|(path, cat)| json!({
            "file": path,
            "category": cat
        })).collect::<Vec<_>>(),
        "dependencies": {
            "total": deps.len(),
            "items": deps.iter().map(|d| json!({
                "name": d.name,
                "version": d.version,
                "ecosystem": d.ecosystem,
                "dev_only": d.dev_only == 1
            })).collect::<Vec<_>>()
        }
    }))
}

async fn resource_project_stats(ctx: &tools::ToolContext) -> anyhow::Result<Value> {
    let domain_stats: Vec<(String, i64)> = ctx.sqlite.get_domain_stats().await?;
    let symbols: Vec<crate::models::chunk::SymbolWithPath> =
        ctx.sqlite.get_symbols(None, None, 0, 10000).await?;

    let errors: Vec<crate::models::chunk::ActiveError> =
        ctx.sqlite.get_errors(None, None, 0, 10000).await?;

    // Count symbols by kind
    let mut kind_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for sym in &symbols {
        *kind_counts.entry(sym.kind.as_str()).or_default() += 1;
    }

    Ok(json!({
        "symbols": {
            "total": symbols.len(),
            "by_kind": kind_counts
        },
        "summaries": 0,
        "errors": errors.len(),
        "domains": domain_stats.iter().map(|(d, c)| json!({
            "domain": d,
            "file_count": c
        })).collect::<Vec<_>>()
    }))
}

// === MCP Prompts ===

async fn handle_prompts_list(id: Value) -> DaemonResponse {
    DaemonResponse::success(
        id,
        json!({
            "prompts": [
                {
                    "name": "review_code",
                    "description": "Generate a code review prompt for a given file, including context bundle and project rules.",
                    "arguments": [
                        { "name": "file", "description": "File to review", "required": true }
                    ]
                },
                {
                    "name": "explain_module",
                    "description": "Generate a prompt to explain a module/file's purpose, dependencies, and architecture.",
                    "arguments": [
                        { "name": "file", "description": "File to explain", "required": true }
                    ]
                },
                {
                    "name": "find_related",
                    "description": "Generate a prompt to find files/code related to a given concept or feature.",
                    "arguments": [
                        { "name": "query", "description": "Concept or feature to find related code for", "required": true }
                    ]
                }
            ]
        }),
    )
}

async fn handle_prompts_get(
    id: Value,
    req: &DaemonRequest,
    state: &Arc<DaemonState>,
) -> DaemonResponse {
    let name = req
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
    let project_path = req.project_path();

    let Some(pp) = project_path else {
        return DaemonResponse::error(id, -32602, "Missing project_path".into());
    };

    let project = match state.get_or_load_project(pp).await {
        Ok(p) => p,
        Err(e) => return DaemonResponse::error(id, -32000, format!("Project load failed: {}", e)),
    };

    let ctx = tools::ToolContext {
        sqlite: Arc::new(project.sqlite.clone()),
        lance: Arc::clone(&project.lance),
        embedder: Arc::clone(&project.embedder),
        root_path: Arc::new(project.path.clone()),
        cache: Arc::clone(&project.cache),
        embedding_circuit: Arc::clone(&state.embedding_circuit),
        vector_circuit: Arc::clone(&state.vector_circuit),
    };

    let result = match name {
        "review_code" => prompt_review_code(&args, &ctx).await,
        "explain_module" => prompt_explain_module(&args, &ctx).await,
        "find_related" => prompt_find_related(&args, &ctx).await,
        _ => {
            return DaemonResponse::error(id, -32602, format!("Unknown prompt: {}", name));
        }
    };

    match result {
        Ok(messages) => DaemonResponse::success(id, json!({ "messages": messages })),
        Err(e) => DaemonResponse::error(id, -32000, format!("Prompt error: {}", e)),
    }
}

async fn prompt_review_code(args: &Value, ctx: &tools::ToolContext) -> anyhow::Result<Vec<Value>> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    if file.is_empty() {
        return Err(anyhow::anyhow!("'file' argument is required"));
    }

    // Get context bundle with skeleton deps
    let bundle: serde_json::Value = tools::dispatch(
        "context_bundle",
        json!({"file": file, "skeleton_deps_only": true, "depth": 2}),
        ctx,
    )
    .await?;

    let rules: Vec<crate::models::chunk::Rule> = ctx.sqlite.get_rules().await?;
    let rules_text = if rules.is_empty() {
        "No project rules defined.".to_string()
    } else {
        rules
            .iter()
            .map(|r| format!("- [{}] (p={}) {}", r.category, r.priority, r.rule))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let bundle_text = serde_json::to_string_pretty(&bundle)?;

    Ok(vec![json!({
        "role": "user",
        "content": {
            "type": "text",
            "text": format!(
                "Review the following code file: `{}`\n\n## Project Rules\n{}\n\n## Context Bundle\n```json\n{}\n```\n\nProvide a thorough code review focusing on:\n1. Correctness and potential bugs\n2. Adherence to project rules\n3. Performance concerns\n4. Security issues\n5. Code style and readability",
                file, rules_text, bundle_text
            )
        }
    })])
}

async fn prompt_explain_module(
    args: &Value,
    ctx: &tools::ToolContext,
) -> anyhow::Result<Vec<Value>> {
    let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
    if file.is_empty() {
        return Err(anyhow::anyhow!("'file' argument is required"));
    }

    let skeleton: serde_json::Value =
        tools::dispatch("skeleton", json!({"file": file}), ctx).await?;
    let symbols: serde_json::Value =
        tools::dispatch("get_symbols", json!({"file": file}), ctx).await?;

    Ok(vec![json!({
        "role": "user",
        "content": {
            "type": "text",
            "text": format!(
                "Explain the module `{}`.\n\n## Skeleton\n```json\n{}\n```\n\n## Symbols\n```json\n{}\n```\n\nProvide:\n1. Overall purpose and responsibility\n2. Key data structures and their roles\n3. Main functions/methods and their flow\n4. Dependencies and how they're used\n5. How this module fits into the larger architecture",
                file,
                serde_json::to_string_pretty(&skeleton)?,
                serde_json::to_string_pretty(&symbols)?
            )
        }
    })])
}

async fn prompt_find_related(args: &Value, ctx: &tools::ToolContext) -> anyhow::Result<Vec<Value>> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.is_empty() {
        return Err(anyhow::anyhow!("'query' argument is required"));
    }

    let search_results: serde_json::Value =
        tools::dispatch("search", json!({"query": query, "limit": 10}), ctx).await?;
    let purpose_results: serde_json::Value = tools::dispatch(
        "search_by_purpose",
        json!({"query": query, "limit": 5}),
        ctx,
    )
    .await?;

    Ok(vec![json!({
        "role": "user",
        "content": {
            "type": "text",
            "text": format!(
                "Find all code related to: \"{}\"\n\n## Semantic Search Results\n```json\n{}\n```\n\n## Purpose-Based Results\n```json\n{}\n```\n\nAnalyze these results and:\n1. Identify the key files involved\n2. Map the data/control flow related to this concept\n3. Identify any missing pieces or gaps\n4. Suggest where changes should be made if modifying this feature",
                query,
                serde_json::to_string_pretty(&search_results)?,
                serde_json::to_string_pretty(&purpose_results)?
            )
        }
    })])
}
