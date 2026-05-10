//! MCP stdio-to-daemon bridge.
//!
//! Proxies JSON-RPC between the MCP client (stdin/stdout) and the gofer daemon
//! (Unix socket). Determines the actual project path via MCP `roots/list` and
//! injects it into every request forwarded to the daemon.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

// Debug logging for MCP communication
fn log_bridge(msg: &str) {
    tracing::debug!("{}", msg);
}

/// Sentinel IDs for internal bridge requests — filtered from client output.
const ROOTS_LIST_ID: u64 = 999999;
const REGISTER_ID: u64 = 999998;
const ACTIVATE_ID: u64 = 999997;

/// Run the MCP bridge: stdin <-> daemon socket.
///
/// `fallback_path` is used as project_path until MCP `roots/list` provides the
/// real workspace root.
pub async fn run_bridge(fallback_path: PathBuf, socket_path: &Path) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut stdin_lines = BufReader::new(stdin).lines();

    let mut project_path = fallback_path.to_string_lossy().to_string();
    let mut roots_requested = false;
    let mut sock_line = String::new();

    // Reconnection loop
    loop {
        // Try to connect to daemon
        let stream = match UnixStream::connect(socket_path).await {
            Ok(s) => s,
            Err(_) => {
                // Daemon not running, wait a bit and retry (MCP client is still alive)
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                continue;
            }
        };

        log_bridge("connected to daemon");
        let (sock_reader, mut sock_writer) = stream.into_split();
        let mut sock_reader = BufReader::new(sock_reader);

        // If we already know the project root (reconnecting), re-register it
        if roots_requested {
            log_bridge("re-registering project after reconnect");
            let _ = send_json(&mut sock_writer, &json!({
                "jsonrpc": "2.0",
                "id": REGISTER_ID,
                "method": "daemon/register_project",
                "params": { "project_path": &project_path }
            })).await;
            let _ = send_json(&mut sock_writer, &json!({
                "jsonrpc": "2.0",
                "id": ACTIVATE_ID,
                "method": "daemon/activate_project",
                "params": {
                    "project_path": &project_path,
                    "watch": true,
                    "background": true
                }
            })).await;
        }

        loop {
            sock_line.clear();

            tokio::select! {
                // ── stdin (MCP client) ──────────────────────────────
                result = stdin_lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            if line.trim().is_empty() {
                                continue;
                            }

                            match serde_json::from_str::<Value>(&line) {
                                Ok(mut msg) => {
                                    let method = msg.get("method")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let msg_id = msg.get("id").cloned();

                                    log_bridge(&format!("recv from client: method={}, id={:?}", method, msg_id));

                                    // ── Response to our roots/list request ──
                                    if is_bridge_id(&msg_id, ROOTS_LIST_ID) {
                                        log_bridge("got roots/list response from client");
                                        if let Some(root) = extract_root_path(&msg) {
                                            project_path = root;
                                            // Auto-register & activate project in daemon
                                            let _ = send_json(&mut sock_writer, &json!({
                                                "jsonrpc": "2.0",
                                                "id": REGISTER_ID,
                                                "method": "daemon/register_project",
                                                "params": { "project_path": &project_path }
                                            })).await;
                                            let _ = send_json(&mut sock_writer, &json!({
                                                "jsonrpc": "2.0",
                                                "id": ACTIVATE_ID,
                                                "method": "daemon/activate_project",
                                                "params": {
                                                    "project_path": &project_path,
                                                    "watch": true,
                                                    "background": true
                                                }
                                            })).await;
                                        }
                                        continue; // don't forward to daemon
                                    }

                                    // ── After "initialized", ask client for roots ──
                                    if (method == "initialized" || method == "notifications/initialized") && !roots_requested {
                                        roots_requested = true;
                                        let _ = send_json(&mut stdout, &json!({
                                            "jsonrpc": "2.0",
                                            "id": ROOTS_LIST_ID,
                                            "method": "roots/list",
                                            "params": {}
                                        })).await;
                                        // Don't forward notification to daemon
                                        // (daemon responds to notifications with id:null
                                        //  which confuses clients)
                                        continue;
                                    }

                                    // ── Filter out other responses from client ──
                                    // If msg does not have a method, it is a response from the client
                                    // We MUST NOT forward client responses to the daemon.
                                    if method.is_empty() {
                                        log_bridge(&format!("swallowing client response: id={:?}", msg_id));
                                        continue;
                                    }

                                    // ── Default: inject project_path, forward ──
                                    log_bridge(&format!("forwarding to daemon: method={}, id={:?}", method, msg_id));
                                    inject_project_path(&mut msg, &project_path);
                                    let _ = send_json(&mut sock_writer, &msg).await;
                                    log_bridge(&format!("forwarded to daemon: method={}", method));
                                }
                                Err(_) => {
                                    // Not valid JSON — forward as-is
                                    let _ = sock_writer.write_all(line.as_bytes()).await;
                                    let _ = sock_writer.write_all(b"\n").await;
                                    let _ = sock_writer.flush().await;
                                }
                            }
                        }
                        // EOF from client (IDE closed MCP connection)
                        Ok(None) | Err(_) => {
                            log_bridge("MCP client disconnected, exiting bridge");
                            return Ok(());
                        }
                    }
                }

                // ── daemon socket → stdout (MCP client) ────────────
                n = sock_reader.read_line(&mut sock_line) => {
                    match n {
                        // EOF or error from daemon socket (daemon disconnected/crashed)
                        Ok(0) | Err(_) => {
                            log_bridge("daemon disconnected, reconnecting...");
                            break; // break inner loop to reconnect
                        }
                        Ok(_) => {
                            log_bridge(&format!("recv from daemon: {}", sock_line.trim()));
                            // Filter out responses to bridge-internal requests
                            if let Ok(resp) = serde_json::from_str::<Value>(&sock_line) {
                                let resp_id = resp.get("id").cloned();
                                let resp_method = resp.get("method").and_then(|v| v.as_str());
                                if is_bridge_id(&resp_id, REGISTER_ID)
                                    || is_bridge_id(&resp_id, ACTIVATE_ID)
                                {
                                    log_bridge("swallowing bridge-internal response");
                                    continue; // swallow
                                }
                                if let Some(method) = resp_method {
                                    log_bridge(&format!("forwarding request from daemon to client: method={}", method));
                                } else {
                                    log_bridge(&format!("forwarding response from daemon to client: id={:?}", resp_id));
                                }
                            }
                            let _ = stdout.write_all(sock_line.as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                    }
                }
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Check whether a JSON-RPC `id` field matches a bridge sentinel u64.
fn is_bridge_id(id: &Option<Value>, sentinel: u64) -> bool {
    id.as_ref()
        .and_then(|v| v.as_u64())
        .map(|s| s == sentinel)
        .unwrap_or(false)
}

/// Extract the first root path from a `roots/list` response.
///
/// Expects: `{"result":{"roots":[{"uri":"file:///path/to/project"}]}}`
fn extract_root_path(response: &Value) -> Option<String> {
    response
        .get("result")
        .and_then(|r| r.get("roots"))
        .and_then(|r| r.as_array())
        .and_then(|roots| roots.first())
        .and_then(|root| root.get("uri"))
        .and_then(|uri| uri.as_str())
        .map(|uri| uri.strip_prefix("file://").unwrap_or(uri).to_string())
}

/// Inject `project_path` into the `params` object of a JSON-RPC request.
fn inject_project_path(req: &mut Value, project_path: &str) {
    if let Some(params) = req.get_mut("params") {
        if let Some(obj) = params.as_object_mut() {
            obj.insert(
                "project_path".to_string(),
                Value::String(project_path.to_string()),
            );
        }
    } else {
        req["params"] = json!({ "project_path": project_path });
    }
}

/// Serialize a JSON value and write it as a single line + flush.
async fn send_json<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, msg: &Value) -> Result<()> {
    let s = serde_json::to_string(msg)?;
    w.write_all(s.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.flush().await?;
    Ok(())
}
