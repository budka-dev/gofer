//! Shared JSON-RPC 2.0 protocol types for daemon IPC.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Incoming JSON-RPC request from clients (CLI, MCP bridge).
#[derive(Debug, Deserialize)]
pub struct DaemonRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl DaemonRequest {
    /// Extract `project_path` from params (injected by the MCP bridge or CLI).
    pub fn project_path(&self) -> Option<&str> {
        self.params.get("project_path").and_then(|v| v.as_str())
    }
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct DaemonResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl DaemonResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i32, message: String) -> Self {
        tracing::error!("RPC Error [{}]: {}", code, message);
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message,
                data: None,
            }),
        }
    }

    /// Build an error response from a [`GoferError`].
    #[allow(dead_code)]
    pub fn from_gofer_error(id: Value, err: crate::error::GoferError) -> Self {
        let (code, message) = err.into_rpc();
        Self::error(id, code, message)
    }
}

/// Server-to-client JSON-RPC notification (no `id` field).
#[derive(Debug, Serialize)]
pub struct DaemonNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: Value,
}

impl DaemonNotification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }

    /// Create an MCP progress notification.
    pub fn progress(token: &str, progress: usize, total: usize, message: &str) -> Self {
        Self::new(
            "notifications/progress",
            serde_json::json!({
                "progressToken": token,
                "progress": progress,
                "total": total,
                "message": message
            }),
        )
    }
}
