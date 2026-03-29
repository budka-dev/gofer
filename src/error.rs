//! Unified error type for gofer daemon — maps internal errors to JSON-RPC codes.

use thiserror::Error;

/// Standard JSON-RPC 2.0 error codes.
const PARSE_ERROR: i32 = -32700;
const INVALID_PARAMS: i32 = -32602;
const METHOD_NOT_FOUND: i32 = -32601;
const INTERNAL_ERROR: i32 = -32603;
/// Application-level server error (implementation-defined).
const SERVER_ERROR: i32 = -32000;

#[derive(Error, Debug)]
pub enum GoferError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid params: {0}")]
    #[allow(dead_code)]
    InvalidParams(String),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Storage error: {0}")]
    Storage(#[from] crate::storage::sqlite::StorageError),

    #[error("Lance error: {0}")]
    Lance(#[from] crate::storage::lance::LanceError),

    #[error("Embedder error: {0}")]
    Embedder(#[from] crate::indexer::embedder::EmbedderError),

    #[error("Parser error: {0}")]
    Parser(#[from] crate::indexer::parser::ParserError),

    #[error("Tool error: {0}")]
    ToolError(String),

    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl GoferError {
    /// JSON-RPC error code for this error variant.
    pub fn rpc_code(&self) -> i32 {
        match self {
            Self::ParseError(_) => PARSE_ERROR,
            Self::InvalidParams(_) => INVALID_PARAMS,
            Self::MethodNotFound(_) => METHOD_NOT_FOUND,
            Self::Storage(_) | Self::Lance(_) | Self::Embedder(_) | Self::Parser(_) | Self::ToolError(_) => SERVER_ERROR,
            Self::Internal(_) => INTERNAL_ERROR,
        }
    }

    /// Convert to (code, message) pair for DaemonResponse::error.
    pub fn into_rpc(self) -> (i32, String) {
        let code = self.rpc_code();
        (code, self.to_string())
    }
}
