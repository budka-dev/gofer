use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};

use super::{LanguageService, ToolDefinition};
use crate::indexer::diagnostics::CargoMessage;
use crate::storage::SqliteStorage;

// ---------------------------------------------------------------------------
// Compiled regex patterns (LazyLock for one-time initialization)
// ---------------------------------------------------------------------------

static TEST_SUMMARY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"test result: (\w+)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored").unwrap()
});

static TEST_FAILURE_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"---- (\S+) stdout ----").unwrap());

pub struct RustService {
    sqlite: SqliteStorage,
}

impl RustService {
    pub fn new(sqlite: SqliteStorage) -> Self {
        Self { sqlite }
    }
}

// ---------------------------------------------------------------------------
// LanguageService impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LanguageService for RustService {
    fn name(&self) -> &str {
        "rust"
    }

    fn is_applicable(&self, root: &Path) -> bool {
        root.join("Cargo.toml").exists()
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        vec![
            // --- Group 1: Comprehension ---
            ToolDefinition {
                name: "rust_project_info".to_string(),
                description: "Get Rust workspace structure via `cargo metadata`: packages, workspace members, features, target directory.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "rust_expand_macro".to_string(),
                description: "Expand Rust macros using `cargo expand`. Returns expanded code or an installation hint if cargo-expand is not installed.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_name": {
                            "type": "string",
                            "description": "Optional: specific item/module to expand (e.g. 'MyStruct' or 'my_module')"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "rust_explain_struct".to_string(),
                description: "Explain a Rust struct: fields, implemented traits, methods, and usage locations (from the project index).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "struct_name": {
                            "type": "string",
                            "description": "Name of the struct to explain"
                        }
                    },
                    "required": ["struct_name"]
                }),
            },
            // --- Group 2: Navigation ---
            ToolDefinition {
                name: "rust_find_trait_impls".to_string(),
                description: "Find all `impl Trait for Type` blocks for a given trait name in the indexed project.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "trait_name": {
                            "type": "string",
                            "description": "Name of the trait to search implementations for"
                        }
                    },
                    "required": ["trait_name"]
                }),
            },
            ToolDefinition {
                name: "rust_resolve_module_path".to_string(),
                description: "Resolve a Rust module path (e.g. `crate::storage::sqlite`) to the physical file path on disk.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "module_path": {
                            "type": "string",
                            "description": "Rust module path starting with 'crate::' (e.g. 'crate::storage::sqlite')"
                        }
                    },
                    "required": ["module_path"]
                }),
            },
            // --- Group 3: Verification ---
            ToolDefinition {
                name: "rust_check_code".to_string(),
                description: "Run `cargo check` and return compiler diagnostics (errors/warnings) with file locations.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Optional: filter diagnostics to this file path"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "rust_clippy".to_string(),
                description: "Run `cargo clippy` and return lint warnings with file locations.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Optional: filter lints to this file path"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "rust_test_run".to_string(),
                description: "Run `cargo test` (optionally a specific test) and return pass/fail results with failure details.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "test_name": {
                            "type": "string",
                            "description": "Optional: run only tests matching this name"
                        }
                    }
                }),
            },
            // --- Group 4: Rust Analyzer (LSP) ---
            ToolDefinition {
                name: "rust_goto_definition".to_string(),
                description: "Go to definition for Rust symbol at position.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "line": { "type": "integer" },
                        "character": { "type": "integer" }
                    },
                    "required": ["file_path", "line", "character"]
                }),
            },
            ToolDefinition {
                name: "rust_find_references".to_string(),
                description: "Find all references to Rust symbol at position.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "line": { "type": "integer" },
                        "character": { "type": "integer" },
                        "include_declaration": { "type": "boolean", "default": true }
                    },
                    "required": ["file_path", "line", "character"]
                }),
            },
            ToolDefinition {
                name: "rust_hover".to_string(),
                description: "Get hover information (type, docs) for Rust symbol at position.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "line": { "type": "integer" },
                        "character": { "type": "integer" }
                    },
                    "required": ["file_path", "line", "character"]
                }),
            },
            ToolDefinition {
                name: "rust_diagnostics".to_string(),
                description: "Get compiler diagnostics (errors, warnings) for a Rust file.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" }
                    },
                    "required": ["file_path"]
                }),
            },
            ToolDefinition {
                name: "rust_completions".to_string(),
                description: "Get code completions for Rust at position.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "line": { "type": "integer" },
                        "character": { "type": "integer" }
                    },
                    "required": ["file_path", "line", "character"]
                }),
            },
            ToolDefinition {
                name: "rust_inlay_hints".to_string(),
                description: "Get inlay hints (type annotations, parameter names) for a Rust file range.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "start_line": { "type": "integer" },
                        "end_line": { "type": "integer" }
                    },
                    "required": ["file_path", "start_line", "end_line"]
                }),
            },
            ToolDefinition {
                name: "rust_code_actions".to_string(),
                description: "Get code actions (quick fixes, refactorings) for a Rust file range.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "start_line": { "type": "integer" },
                        "end_line": { "type": "integer" }
                    },
                    "required": ["file_path", "start_line", "end_line"]
                }),
            },
        ]
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        ctx: &crate::daemon::tools::ToolContext,
    ) -> anyhow::Result<String> {
        let root = ctx.root_path.as_path();
        match name {
            "rust_project_info" => self.tool_project_info(root).await,
            "rust_expand_macro" => self.tool_expand_macro(args, root).await,
            "rust_explain_struct" => self.tool_explain_struct(args).await,
            "rust_find_trait_impls" => self.tool_find_trait_impls(args).await,
            "rust_resolve_module_path" => self.tool_resolve_module_path(args, root).await,
            "rust_check_code" => self.tool_check_code(args, root).await,
            "rust_clippy" => self.tool_clippy(args, root).await,
            "rust_test_run" => self.tool_test_run(args, root).await,

            // Rust Analyzer (LSP) Tools
            "rust_goto_definition" => {
                let res =
                    crate::daemon::handlers::lsp::tool_lsp_goto_definition(args, ctx)
                        .await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "rust_find_references" => {
                let res =
                    crate::daemon::handlers::lsp::tool_lsp_find_references(args, ctx)
                        .await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "rust_hover" => {
                let res =
                    crate::daemon::handlers::lsp::tool_lsp_hover(args, ctx).await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "rust_diagnostics" => {
                let res = crate::daemon::handlers::lsp::tool_lsp_diagnostics(args, ctx)
                    .await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "rust_completions" => {
                let res = crate::daemon::handlers::lsp::tool_lsp_completions(args, ctx)
                    .await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "rust_inlay_hints" => {
                let res = crate::daemon::handlers::lsp::tool_lsp_inlay_hints(args, ctx)
                    .await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "rust_code_actions" => {
                let res = crate::daemon::handlers::lsp::tool_lsp_code_actions(args, ctx)
                    .await?;
                Ok(serde_json::to_string_pretty(&res)?)
            }

            _ => Err(anyhow::anyhow!("Unknown Rust tool: {}", name)),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

struct DiagnosticEntry {
    level: String,
    file: String,
    line: u32,
    code: Option<String>,
    message: String,
}

fn parse_cargo_diagnostics(stdout: &str, file_filter: Option<&str>) -> Vec<DiagnosticEntry> {
    let mut entries = Vec::new();

    for line in stdout.lines() {
        let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
            continue;
        };
        if msg.reason != "compiler-message" {
            continue;
        }
        let Some(ref cm) = msg.message else {
            continue;
        };

        for span in &cm.spans {
            if !span.is_primary {
                continue;
            }
            if let Some(filter) = file_filter {
                if !span.file_name.ends_with(filter) && !span.file_name.contains(filter) {
                    continue;
                }
            }
            entries.push(DiagnosticEntry {
                level: cm.level.clone(),
                file: span.file_name.clone(),
                line: span.line_start,
                code: cm.code.as_ref().map(|c| c.code.clone()),
                message: cm.message.clone(),
            });
        }
    }

    entries
}

fn format_diagnostics(entries: &[DiagnosticEntry], header: &str) -> String {
    let mut out = format!("# {}\n\n", header);

    if entries.is_empty() {
        out.push_str("No errors or warnings.\n");
        return out;
    }

    let errors = entries.iter().filter(|e| e.level == "error").count();
    let warnings = entries.iter().filter(|e| e.level == "warning").count();
    out.push_str(&format!(
        "Found {} error(s), {} warning(s)\n\n",
        errors, warnings
    ));

    for e in entries {
        let code_str = e.code.as_deref().unwrap_or("");
        if code_str.is_empty() {
            out.push_str(&format!(
                "- **[{}]** `{}:{}`: {}\n",
                e.level.to_uppercase(),
                e.file,
                e.line,
                e.message
            ));
        } else {
            out.push_str(&format!(
                "- **[{}]** `{}:{}` ({}): {}\n",
                e.level.to_uppercase(),
                e.file,
                e.line,
                code_str,
                e.message
            ));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

impl RustService {
    /// `rust_project_info` — cargo metadata
    async fn tool_project_info(&self, root: &Path) -> Result<String> {
        let output = tokio::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .current_dir(root)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("cargo metadata failed: {}", stderr.trim()));
        }

        let meta: Value = serde_json::from_slice(&output.stdout)?;

        let workspace_root = meta["workspace_root"].as_str().unwrap_or("?");
        let target_dir = meta["target_directory"].as_str().unwrap_or("?");

        let members = meta["workspace_members"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);

        let mut out = String::from("# Rust Project Info\n\n");
        out.push_str(&format!("- **Workspace root:** `{}`\n", workspace_root));
        out.push_str(&format!("- **Target directory:** `{}`\n", target_dir));
        out.push_str(&format!("- **Workspace members:** {}\n\n", members));

        if let Some(packages) = meta["packages"].as_array() {
            out.push_str("## Packages\n\n");
            for pkg in packages {
                let name = pkg["name"].as_str().unwrap_or("?");
                let version = pkg["version"].as_str().unwrap_or("?");
                let edition = pkg["edition"].as_str().unwrap_or("2021");
                out.push_str(&format!(
                    "### `{}` v{} (edition {})\n\n",
                    name, version, edition
                ));

                // Dependencies
                if let Some(deps) = pkg["dependencies"].as_array() {
                    out.push_str(&format!("**Dependencies ({}):**\n", deps.len()));
                    for dep in deps {
                        let dname = dep["name"].as_str().unwrap_or("?");
                        let req = dep["req"].as_str().unwrap_or("*");
                        let optional = dep["optional"].as_bool().unwrap_or(false);
                        let marker = if optional { " (optional)" } else { "" };
                        out.push_str(&format!("- `{}` {}{}\n", dname, req, marker));
                    }
                    out.push('\n');
                }

                // Features
                if let Some(features) = pkg["features"].as_object() {
                    if !features.is_empty() {
                        out.push_str("**Features:**\n");
                        for (fname, fdeps) in features {
                            let deps_str = fdeps
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v| v.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default();
                            if deps_str.is_empty() {
                                out.push_str(&format!("- `{}`\n", fname));
                            } else {
                                out.push_str(&format!("- `{}` = [{}]\n", fname, deps_str));
                            }
                        }
                        out.push('\n');
                    }
                }
            }
        }

        Ok(out)
    }

    /// `rust_expand_macro` — cargo expand (best effort)
    async fn tool_expand_macro(&self, args: Value, root: &Path) -> Result<String> {
        let item_name = args.get("item_name").and_then(|v| v.as_str());

        // Check if cargo-expand is available
        let check = tokio::process::Command::new("cargo")
            .args(["expand", "--version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        let is_available = check.map(|s| s.success()).unwrap_or(false);
        if !is_available {
            return Ok("**cargo-expand is not installed.**\n\n\
                 Install it with:\n```\ncargo install cargo-expand\n```\n\
                 Then re-run this tool.".to_string());
        }

        let mut cmd = tokio::process::Command::new("cargo");
        cmd.arg("expand").current_dir(root);

        if let Some(item) = item_name {
            cmd.arg(item);
        }

        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("cargo expand failed: {}", stderr.trim()));
        }

        let expanded = String::from_utf8_lossy(&output.stdout);
        let label = item_name.unwrap_or("(full crate)");
        Ok(format!(
            "# Macro Expansion: {}\n\n```rust\n{}\n```\n",
            label,
            expanded.trim()
        ))
    }

    /// `rust_explain_struct` — struct fields, impls, methods from index
    async fn tool_explain_struct(&self, args: Value) -> Result<String> {
        let struct_name = args
            .get("struct_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'struct_name' is required"))?;

        // Find struct symbol
        let symbols = self.sqlite.get_symbol_by_name(struct_name).await?;
        let struct_sym = symbols
            .iter()
            .find(|s| s.kind == crate::models::chunk::SymbolKind::Struct)
            .ok_or_else(|| anyhow::anyhow!("Struct '{}' not found in index", struct_name))?;

        let file = self
            .sqlite
            .get_file_by_id(struct_sym.file_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("File not found for struct"))?;

        let mut out = format!("# Struct: `{}`\n\n", struct_name);
        out.push_str(&format!(
            "**Location:** `{}:{}`\n\n",
            file.path, struct_sym.line_start
        ));

        if let Some(ref sig) = struct_sym.signature {
            out.push_str(&format!("```rust\n{}\n```\n\n", sig));
        }

        // Find all impl blocks and methods in the same file
        let all_symbols = self.sqlite.get_file_symbols(struct_sym.file_id).await?;

        let impl_blocks: Vec<_> = all_symbols
            .iter()
            .filter(|s| s.kind == crate::models::chunk::SymbolKind::Impl && s.name == struct_name)
            .collect();

        if !impl_blocks.is_empty() {
            out.push_str("## Implementations\n\n");
            for ib in &impl_blocks {
                let default_header = format!("impl {}", struct_name);
                let header = ib.signature.as_deref().unwrap_or(&default_header);
                out.push_str(&format!("### `{}` (line {})\n\n", header, ib.line_start));

                // Methods inside this impl block (by line range)
                let methods: Vec<_> = all_symbols
                    .iter()
                    .filter(|s| {
                        s.kind == crate::models::chunk::SymbolKind::Function
                            && s.line_start >= ib.line_start
                            && s.line_end <= ib.line_end
                    })
                    .collect();

                if methods.is_empty() {
                    out.push_str("*(no methods)*\n\n");
                } else {
                    for m in &methods {
                        let sig = m.signature.as_deref().unwrap_or(&m.name);
                        out.push_str(&format!("- `{}` (line {})\n", sig, m.line_start));
                    }
                    out.push('\n');
                }
            }
        }

        // Also find trait impls that reference this struct
        // (impl blocks where signature contains "for StructName")
        let trait_impls: Vec<_> = all_symbols
            .iter()
            .filter(|s| {
                s.kind == crate::models::chunk::SymbolKind::Impl
                    && s.name != struct_name
                    && s.signature
                        .as_ref()
                        .map(|sig| sig.contains(&format!("for {}", struct_name)))
                        .unwrap_or(false)
            })
            .collect();

        if !trait_impls.is_empty() {
            out.push_str("## Trait Implementations\n\n");
            for ti in &trait_impls {
                let sig = ti.signature.as_deref().unwrap_or(&ti.name);
                out.push_str(&format!("- `{}` (line {})\n", sig, ti.line_start));
            }
            out.push('\n');
        }

        // References
        let refs = self.sqlite.get_incoming_references(struct_name).await?;
        if !refs.is_empty() {
            out.push_str(&format!(
                "## References\n\nUsed by **{}** symbol(s).\n",
                refs.len()
            ));
        }

        Ok(out)
    }

    /// `rust_find_trait_impls` — find impl blocks for a trait
    async fn tool_find_trait_impls(&self, args: Value) -> Result<String> {
        let trait_name = args
            .get("trait_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'trait_name' is required"))?;

        // Query all symbols named after the trait — impl blocks may store the
        // trait name when it's `impl Trait for Type`.
        let symbols = self.sqlite.get_symbol_by_name(trait_name).await?;
        let impls: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == crate::models::chunk::SymbolKind::Impl)
            .collect();

        let mut out = format!("# Implementations of `{}`\n\n", trait_name);

        if impls.is_empty() {
            out.push_str("No implementations found in the project index.\n\n");
            out.push_str("*Tip: make sure the project has been indexed (`gofer index sync`).*\n");
            return Ok(out);
        }

        for sym in &impls {
            let file = self.sqlite.get_file_by_id(sym.file_id).await?;
            let path = file.map(|f| f.path).unwrap_or_else(|| "?".to_string());
            let sig = sym.signature.as_deref().unwrap_or(&sym.name);
            out.push_str(&format!("- `{}` at `{}:{}`\n", sig, path, sym.line_start));
        }

        Ok(out)
    }

    /// `rust_resolve_module_path` — crate::a::b → src/a/b.rs
    async fn tool_resolve_module_path(&self, args: Value, root: &Path) -> Result<String> {
        let module_path = args
            .get("module_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'module_path' is required"))?;

        let parts: Vec<&str> = module_path.split("::").collect();

        if parts.is_empty() || (parts[0] != "crate" && parts[0] != "self" && parts[0] != "super") {
            return Err(anyhow::anyhow!(
                "Module path should start with 'crate::' (e.g. 'crate::storage::sqlite')"
            ));
        }

        let src_dir = root.join("src");
        let segments = &parts[1..]; // skip "crate"

        let mut candidates: Vec<PathBuf> = Vec::new();

        // Build all possible paths
        if segments.is_empty() {
            // "crate" alone → src/lib.rs or src/main.rs
            let lib = src_dir.join("lib.rs");
            let main = src_dir.join("main.rs");
            if lib.exists() {
                candidates.push(lib);
            }
            if main.exists() {
                candidates.push(main);
            }
        } else {
            // crate::a::b::c  →  try src/a/b/c.rs  and  src/a/b/c/mod.rs
            let mut rel = PathBuf::new();
            for seg in segments {
                rel.push(seg);
            }

            let file_variant = src_dir.join(&rel).with_extension("rs");
            let mod_variant = src_dir.join(&rel).join("mod.rs");

            if file_variant.exists() {
                candidates.push(file_variant);
            }
            if mod_variant.exists() {
                candidates.push(mod_variant);
            }
        }

        let mut out = format!("# Module: `{}`\n\n", module_path);

        if candidates.is_empty() {
            out.push_str("No matching file found on disk.\n\n");
            // Show what we looked for
            if !segments.is_empty() {
                let mut rel = PathBuf::new();
                for seg in segments {
                    rel.push(seg);
                }
                out.push_str("Searched:\n");
                out.push_str(&format!(
                    "- `src/{}.rs`\n- `src/{}/mod.rs`\n",
                    rel.display(),
                    rel.display()
                ));
            }
        } else {
            out.push_str("**Resolved to:**\n\n");
            for c in &candidates {
                let display = c.strip_prefix(root).unwrap_or(c).display();
                out.push_str(&format!("- `{}`\n", display));
            }
        }

        Ok(out)
    }

    /// `rust_check_code` — cargo check
    async fn tool_check_code(&self, args: Value, root: &Path) -> Result<String> {
        let file_filter = args.get("file").and_then(|v| v.as_str());

        let output = tokio::process::Command::new("cargo")
            .args(["check", "--message-format=json", "--quiet"])
            .current_dir(root)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let entries = parse_cargo_diagnostics(&stdout, file_filter);

        Ok(format_diagnostics(&entries, "Cargo Check"))
    }

    /// `rust_clippy` — cargo clippy
    async fn tool_clippy(&self, args: Value, root: &Path) -> Result<String> {
        let file_filter = args.get("file").and_then(|v| v.as_str());

        // Check clippy availability
        let check = tokio::process::Command::new("cargo")
            .args(["clippy", "--version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        let is_available = check.map(|s| s.success()).unwrap_or(false);
        if !is_available {
            return Ok("**clippy is not installed.**\n\n\
                 Install it with:\n```\nrustup component add clippy\n```\n".to_string());
        }

        let output = tokio::process::Command::new("cargo")
            .args(["clippy", "--message-format=json", "--quiet"])
            .current_dir(root)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let entries = parse_cargo_diagnostics(&stdout, file_filter);

        Ok(format_diagnostics(&entries, "Clippy Lints"))
    }

    /// `rust_test_run` — cargo test
    async fn tool_test_run(&self, args: Value, root: &Path) -> Result<String> {
        let test_name = args.get("test_name").and_then(|v| v.as_str());

        let mut cmd = tokio::process::Command::new("cargo");
        cmd.arg("test").current_dir(root);

        if let Some(name) = test_name {
            cmd.arg(name);
        }

        cmd.arg("--").arg("--nocapture");

        let output = cmd.output().await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut out = String::from("# Cargo Test Results\n\n");

        // Parse summary line: "test result: ok. X passed; Y failed; Z ignored"
        let combined = format!("{}\n{}", stdout, stderr);

        if let Some(caps) = TEST_SUMMARY_RE.captures(&combined) {
            let result = caps.get(1).map(|m| m.as_str()).unwrap_or("?");
            let passed = caps.get(2).map(|m| m.as_str()).unwrap_or("0");
            let failed = caps.get(3).map(|m| m.as_str()).unwrap_or("0");
            let ignored = caps.get(4).map(|m| m.as_str()).unwrap_or("0");

            out.push_str(&format!(
                "**Result:** {}\n- Passed: {}\n- Failed: {}\n- Ignored: {}\n\n",
                result.to_uppercase(),
                passed,
                failed,
                ignored
            ));

            // Extract failure details
            if failed != "0" {
                out.push_str("## Failures\n\n");
                // Pattern: "---- test_name stdout ----" ... "failures:"
                let sections: Vec<&str> = combined.split("---- ").collect();
                for section in &sections[1..] {
                    if let Some(hcaps) =
                        TEST_FAILURE_HEADER_RE.captures(&format!("---- {}", section))
                    {
                        let tname = hcaps.get(1).map(|m| m.as_str()).unwrap_or("?");
                        // Get content up to the next separator
                        let body = section
                            .lines()
                            .skip(1) // skip the header line
                            .take_while(|l| !l.starts_with("----") && !l.starts_with("failures:"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !body.trim().is_empty() {
                            out.push_str(&format!(
                                "### `{}`\n\n```\n{}\n```\n\n",
                                tname,
                                body.trim()
                            ));
                        }
                    }
                }
            }
        } else {
            // Could not parse summary — show raw output (truncated)
            let truncated: String = combined.chars().take(3000).collect();
            out.push_str(&format!("```\n{}\n```\n", truncated.trim()));
        }

        Ok(out)
    }
}
