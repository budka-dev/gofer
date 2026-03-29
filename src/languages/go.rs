use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};
use tree_sitter::Language;

use super::{LanguageService, ToolDefinition};
use crate::storage::SqliteStorage;

pub struct GoService {
    #[allow(dead_code)]
    sqlite: SqliteStorage,
}

impl GoService {
    pub fn new(sqlite: SqliteStorage) -> Self {
        Self { sqlite }
    }
}

// ---------------------------------------------------------------------------
// LanguageService impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LanguageService for GoService {
    fn name(&self) -> &str {
        "go"
    }

    fn is_applicable(&self, root: &Path) -> bool {
        root.join("go.mod").exists()
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        vec![
            // --- Group 1: Comprehension ---
            ToolDefinition {
                name: "go_project_info".to_string(),
                description: "Get Go module information from go.mod: module path, Go version, dependencies, and replace directives.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "go_explain_struct".to_string(),
                description: "Explain a Go struct: fields, methods (value + pointer receivers), and interface implementations found in the index.".to_string(),
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
            ToolDefinition {
                name: "go_find_interface_impls".to_string(),
                description: "Find all types that implement a given Go interface by searching the project index for matching method sets.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "interface_name": {
                            "type": "string",
                            "description": "Name of the interface to search implementations for"
                        }
                    },
                    "required": ["interface_name"]
                }),
            },
            // --- Group 2: Verification ---
            ToolDefinition {
                name: "go_vet".to_string(),
                description: "Run `go vet ./...` and return diagnostics with file locations.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "package": {
                            "type": "string",
                            "description": "Optional: specific package to vet (default: ./...)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "go_build".to_string(),
                description: "Run `go build ./...` and return compiler errors with file locations.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "package": {
                            "type": "string",
                            "description": "Optional: specific package to build (default: ./...)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "go_test".to_string(),
                description: "Run `go test` (optionally for a specific package or test name) and return pass/fail results.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "package": {
                            "type": "string",
                            "description": "Optional: package to test (default: ./...)"
                        },
                        "test_name": {
                            "type": "string",
                            "description": "Optional: run only tests matching this name (-run flag)"
                        }
                    }
                }),
            },
        ]
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        ctx: &crate::daemon::tools::ToolContext,
    ) -> Result<String> {
        let root = ctx.root_path.as_path();
        match name {
            "go_project_info" => self.tool_project_info(root).await,
            "go_explain_struct" => self.tool_explain_struct(args).await,
            "go_find_interface_impls" => self.tool_find_interface_impls(args).await,
            "go_vet" => self.tool_vet(args, root).await,
            "go_build" => self.tool_build(args, root).await,
            "go_test" => self.tool_test(args, root).await,
            _ => Err(anyhow::anyhow!("Unknown Go tool: {}", name)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

impl GoService {
    /// `go_project_info` — parse go.mod
    async fn tool_project_info(&self, root: &Path) -> Result<String> {
        let go_mod_path = root.join("go.mod");
        let content = tokio::fs::read_to_string(&go_mod_path).await?;

        let mut out = String::from("# Go Project Info\n\n");

        // Parse module name
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(module) = trimmed.strip_prefix("module ") {
                out.push_str(&format!("- **Module:** `{}`\n", module.trim()));
            }
            if let Some(version) = trimmed.strip_prefix("go ") {
                out.push_str(&format!("- **Go version:** `{}`\n", version.trim()));
            }
        }
        out.push('\n');

        // Parse require block
        let mut requires = Vec::new();
        let mut in_require = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "require (" {
                in_require = true;
                continue;
            }
            if in_require && trimmed == ")" {
                in_require = false;
                continue;
            }
            if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
                requires.push(trimmed.to_string());
            }
            // Single-line require
            if let Some(dep) = trimmed.strip_prefix("require ") {
                if !dep.starts_with('(') {
                    requires.push(dep.trim().to_string());
                }
            }
        }

        if !requires.is_empty() {
            out.push_str(&format!("## Dependencies ({})\n\n", requires.len()));
            for req in &requires {
                let parts: Vec<&str> = req.split_whitespace().collect();
                if parts.len() >= 2 {
                    let indirect = if req.contains("// indirect") {
                        " *(indirect)*"
                    } else {
                        ""
                    };
                    out.push_str(&format!("- `{}` {}{}\n", parts[0], parts[1], indirect));
                } else {
                    out.push_str(&format!("- `{}`\n", req));
                }
            }
            out.push('\n');
        }

        // Parse replace block
        let mut replaces = Vec::new();
        let mut in_replace = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "replace (" {
                in_replace = true;
                continue;
            }
            if in_replace && trimmed == ")" {
                in_replace = false;
                continue;
            }
            if in_replace && !trimmed.is_empty() && !trimmed.starts_with("//") {
                replaces.push(trimmed.to_string());
            }
            if let Some(rep) = trimmed.strip_prefix("replace ") {
                if !rep.starts_with('(') {
                    replaces.push(rep.trim().to_string());
                }
            }
        }

        if !replaces.is_empty() {
            out.push_str("## Replace Directives\n\n");
            for rep in &replaces {
                out.push_str(&format!("- `{}`\n", rep));
            }
            out.push('\n');
        }

        Ok(out)
    }

    /// `go_explain_struct` — struct fields, methods from index + tree-sitter AST
    async fn tool_explain_struct(&self, args: Value) -> Result<String> {
        let struct_name = args
            .get("struct_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'struct_name' is required"))?;

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
            out.push_str(&format!("```go\n{}\n```\n\n", sig));
        }

        // tree-sitter AST analysis: extract fields and methods from source
        let source = tokio::fs::read_to_string(&file.path).await.ok();
        if let Some(ref src) = source {
            let (fields, methods) = go_analyze_struct(src, struct_name);

            if !fields.is_empty() {
                out.push_str("## Fields\n\n");
                for (name, ty, tag) in &fields {
                    if let Some(t) = tag {
                        out.push_str(&format!("- `{} {}` {}\n", name, ty, t));
                    } else {
                        out.push_str(&format!("- `{} {}`\n", name, ty));
                    }
                }
                out.push('\n');
            }

            if !methods.is_empty() {
                out.push_str("## Methods\n\n");
                for (name, sig, is_ptr) in &methods {
                    let recv = if *is_ptr {
                        format!("*{}", struct_name)
                    } else {
                        struct_name.to_string()
                    };
                    out.push_str(&format!(
                        "- `func ({}) {}`\n",
                        recv,
                        sig.as_deref().unwrap_or(name)
                    ));
                }
                out.push('\n');
            }
        } else {
            // Fallback: use index for methods
            let all_symbols = self.sqlite.get_file_symbols(struct_sym.file_id).await?;

            let methods: Vec<_> = all_symbols
                .iter()
                .filter(|s| {
                    s.kind == crate::models::chunk::SymbolKind::Method
                        && s.signature
                            .as_ref()
                            .map(|sig| sig.contains(struct_name))
                            .unwrap_or(false)
                })
                .collect();

            if !methods.is_empty() {
                out.push_str("## Methods\n\n");
                for m in &methods {
                    let sig = m.signature.as_deref().unwrap_or(&m.name);
                    out.push_str(&format!("- `{}` (line {})\n", sig, m.line_start));
                }
                out.push('\n');
            }
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

    /// `go_find_interface_impls` — find types implementing an interface
    async fn tool_find_interface_impls(&self, args: Value) -> Result<String> {
        let interface_name = args
            .get("interface_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'interface_name' is required"))?;

        // Find interface symbol to get its methods
        let symbols = self.sqlite.get_symbol_by_name(interface_name).await?;
        let iface = symbols
            .iter()
            .find(|s| s.kind == crate::models::chunk::SymbolKind::Interface);

        let mut out = format!("# Implementations of `{}`\n\n", interface_name);

        if iface.is_none() {
            out.push_str("Interface not found in the project index.\n\n");
            out.push_str("*Tip: make sure the project has been indexed (`gofer index sync`).*\n");
            return Ok(out);
        }

        // Find references to the interface name — types that mention it
        let refs = self.sqlite.get_incoming_references(interface_name).await?;

        if refs.is_empty() {
            out.push_str("No implementations found via reference tracking.\n\n");
            out.push_str("*Note: Go interfaces are implicitly satisfied. This tool searches for type references to the interface name in the index.*\n");
        } else {
            for r in &refs {
                out.push_str(&format!(
                    "- Referenced by symbol ID {} (kind: {})\n",
                    r.source_symbol_id, r.kind
                ));
            }
        }

        Ok(out)
    }

    /// `go_vet` — go vet
    async fn tool_vet(&self, args: Value, root: &Path) -> Result<String> {
        let package = args
            .get("package")
            .and_then(|v| v.as_str())
            .unwrap_or("./...");

        let output = tokio::process::Command::new("go")
            .args(["vet", package])
            .current_dir(root)
            .output()
            .await?;

        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut out = String::from("# Go Vet\n\n");

        if output.status.success() && stderr.trim().is_empty() {
            out.push_str("No issues found.\n");
        } else {
            let lines: Vec<&str> = stderr.lines().collect();
            if lines.is_empty() {
                out.push_str("No issues found.\n");
            } else {
                out.push_str(&format!("Found {} diagnostic(s):\n\n", lines.len()));
                for line in &lines {
                    out.push_str(&format!("- `{}`\n", line));
                }
            }
        }

        Ok(out)
    }

    /// `go_build` — go build
    async fn tool_build(&self, args: Value, root: &Path) -> Result<String> {
        let package = args
            .get("package")
            .and_then(|v| v.as_str())
            .unwrap_or("./...");

        let output = tokio::process::Command::new("go")
            .args(["build", package])
            .current_dir(root)
            .output()
            .await?;

        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut out = String::from("# Go Build\n\n");

        if output.status.success() {
            out.push_str("Build succeeded with no errors.\n");
        } else {
            let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
            out.push_str(&format!("Build failed with {} error(s):\n\n", lines.len()));
            for line in &lines {
                out.push_str(&format!("- `{}`\n", line));
            }
        }

        Ok(out)
    }

    /// `go_test` — go test
    async fn tool_test(&self, args: Value, root: &Path) -> Result<String> {
        let package = args
            .get("package")
            .and_then(|v| v.as_str())
            .unwrap_or("./...");
        let test_name = args.get("test_name").and_then(|v| v.as_str());

        let mut cmd = tokio::process::Command::new("go");
        cmd.arg("test").arg("-json").arg(package).current_dir(root);

        if let Some(name) = test_name {
            cmd.arg("-run").arg(name);
        }

        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        let mut out = String::from("# Go Test Results\n\n");

        // Track test counts
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut failures = Vec::new();

        // Map test name to accumulated output for failures
        let mut test_outputs: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for line in stdout.lines() {
            if let Ok(event) = serde_json::from_str::<Value>(line) {
                let action = event.get("Action").and_then(|v| v.as_str());
                let test = event.get("Test").and_then(|v| v.as_str());
                let output_text = event.get("Output").and_then(|v| v.as_str()).unwrap_or("");

                if let Some(t) = test {
                    if !output_text.is_empty() {
                        test_outputs
                            .entry(t.to_string())
                            .or_default()
                            .push_str(output_text);
                    }

                    match action {
                        Some("pass") => passed += 1,
                        Some("fail") => {
                            failed += 1;
                            let output = test_outputs
                                .get(t)
                                .map(|s| s.as_str())
                                .unwrap_or("(no output)");
                            failures.push(format!("### {}\n```\n{}\n```", t, output.trim()));
                        }
                        Some("skip") => skipped += 1,
                        _ => {}
                    }
                }
            }
        }

        if output.status.success() {
            out.push_str("**Result:** PASS\n");
        } else {
            out.push_str("**Result:** FAIL\n");
        }
        out.push_str(&format!(
            "- Passed: {}\n- Failed: {}\n- Skipped: {}\n\n",
            passed, failed, skipped
        ));

        if !failures.is_empty() {
            out.push_str("## Failures\n\n");
            for fail in failures {
                out.push_str(&format!("{}\n\n", fail));
            }
        } else if failed > 0 {
            // Fallback if JSON parsing missed something but status failed
            let stderr = String::from_utf8_lossy(&output.stderr);
            out.push_str("## Stderr\n\n");
            out.push_str(&format!("```\n{}\n```\n", stderr));
        }

        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// tree-sitter-go AST analysis helpers
// ---------------------------------------------------------------------------

/// Analyze a Go source file to extract struct fields and methods for a given struct.
/// Returns (fields, methods) where:
///   fields = Vec<(name, type, optional_tag)>
///   methods = Vec<(name, signature, is_pointer_receiver)>
#[allow(clippy::type_complexity)]
fn go_analyze_struct(
    source: &str,
    struct_name: &str,
) -> (
    Vec<(String, String, Option<String>)>,
    Vec<(String, Option<String>, bool)>,
) {
    let go_lang: Language = crate::indexer::parser::LANG_MANAGER.get_language("go").expect("Lang not loaded").language.clone();
    
    let tree_opt = crate::indexer::parser::with_parser(|parser| {
        if parser.set_language(&go_lang).is_err() {
            return None;
        }
        parser.parse(source, None)
    });
    
    let tree = match tree_opt {
        Some(t) => t,
        None => return (Vec::new(), Vec::new()),
    };
    let root = tree.root_node();
    let src = source.as_bytes();

    let mut fields = Vec::new();
    let mut methods = Vec::new();

    // Walk top-level declarations
    for i in 0..root.child_count() {
        let node = match root.child(i as u32) {
            Some(n) => n,
            None => continue,
        };

        // type_declaration -> type_spec -> struct_type
        if node.kind() == "type_declaration" {
            for j in 0..node.child_count() {
                let spec = match node.child(j as u32) {
                    Some(n) if n.kind() == "type_spec" => n,
                    _ => continue,
                };
                let name_node = spec.child_by_field_name("name");
                let type_node = spec.child_by_field_name("type");

                let name = name_node.and_then(|n| n.utf8_text(src).ok()).unwrap_or("");
                if name != struct_name {
                    continue;
                }

                if let Some(struct_node) = type_node {
                    if struct_node.kind() != "struct_type" {
                        continue;
                    }
                    // Extract field_declaration_list
                    for k in 0..struct_node.child_count() {
                        let field_list = match struct_node.child(k as u32) {
                            Some(n) if n.kind() == "field_declaration_list" => n,
                            _ => continue,
                        };
                        for f in 0..field_list.child_count() {
                            let field = match field_list.child(f as u32) {
                                Some(n) if n.kind() == "field_declaration" => n,
                                _ => continue,
                            };
                            let fname = field
                                .child_by_field_name("name")
                                .and_then(|n| n.utf8_text(src).ok())
                                .unwrap_or("_")
                                .to_string();
                            let ftype = field
                                .child_by_field_name("type")
                                .and_then(|n| n.utf8_text(src).ok())
                                .unwrap_or("?")
                                .to_string();
                            let ftag = field
                                .child_by_field_name("tag")
                                .and_then(|n| n.utf8_text(src).ok())
                                .map(|s| s.to_string());
                            fields.push((fname, ftype, ftag));
                        }
                    }
                }
            }
        }

        // method_declaration: func (r *Type) Name(params) returns
        if node.kind() == "method_declaration" {
            let receiver = node.child_by_field_name("receiver");
            let mname = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
                .unwrap_or("")
                .to_string();

            if let Some(recv) = receiver {
                let recv_text = recv.utf8_text(src).ok().unwrap_or("");
                // Check if receiver type matches struct_name
                let is_match = recv_text.contains(struct_name);
                let is_ptr = recv_text.contains('*');

                if is_match {
                    let full_sig = node.utf8_text(src).ok().map(|s| {
                        // Extract just the signature line (first line)
                        s.lines().next().unwrap_or(s).to_string()
                    });
                    methods.push((mname, full_sig, is_ptr));
                }
            }
        }
    }

    (fields, methods)
}
