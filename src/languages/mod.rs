pub mod go;
pub mod python;
pub mod rust;
pub mod generic_lsp;
pub mod typescript;
pub mod vue;

use serde_json::Value;
use std::path::Path;

/// Shared tool definition for MCP registration
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Language-specific service trait.
/// Each implementation provides a set of MCP tools tailored for a specific
/// language/toolchain (Rust, TypeScript, Python, etc.).
#[async_trait::async_trait]
pub trait LanguageService: Send + Sync {
    /// Service identifier (e.g. "rust", "typescript")
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Whether this service is applicable to the given project root
    fn is_applicable(&self, root: &Path) -> bool;

    /// MCP tool definitions this service provides
    fn tools(&self) -> Vec<ToolDefinition>;

    /// Execute a tool by name
    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        ctx: &crate::daemon::tools::ToolContext,
    ) -> anyhow::Result<String>;
}
