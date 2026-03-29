use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{LanguageService, ToolDefinition};
use crate::storage::SqliteStorage;

// ---------------------------------------------------------------------------
// Compiled regex patterns (LazyLock for one-time initialization)
// ---------------------------------------------------------------------------

static TSC_DIAGNOSTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+?)\((\d+),(\d+)\):\s*(error|warning)\s+(TS\d+):\s*(.+)$").unwrap()
});

// ---------------------------------------------------------------------------
// tsconfig.json models
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TsConfig {
    #[serde(alias = "compilerOptions")]
    compiler_options: CompilerOptions,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CompilerOptions {
    #[serde(alias = "baseUrl")]
    base_url: Option<String>,
    paths: Option<HashMap<String, Vec<String>>>,
}

/// Parsed and normalised path alias: prefix -> list of replacement roots
#[derive(Debug, Clone)]
struct PathAlias {
    prefix: String,            // e.g. "@/"
    replacements: Vec<String>, // e.g. ["src/"]
}

fn load_tsconfig_aliases(root: &Path) -> Vec<PathAlias> {
    let candidates = ["tsconfig.json", "tsconfig.app.json", "jsconfig.json"];
    for c in &candidates {
        let path = root.join(c);
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                // Strip single-line comments (tsconfig allows them)
                let cleaned = strip_json_comments(&raw);
                if let Ok(cfg) = serde_json::from_str::<TsConfig>(&cleaned) {
                    return build_aliases(&cfg.compiler_options, root);
                }
            }
        }
    }
    Vec::new()
}

/// Strip // and /* */ comments from JSON (tsconfig allows them)
fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some(&next) = chars.peek() {
                    out.push(next);
                    chars.next();
                }
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek() {
                Some(&'/') => {
                    // Line comment — skip until newline
                    for c in chars.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some(&'*') => {
                    // Block comment — skip until */
                    chars.next(); // consume *
                    let mut prev = ' ';
                    for c in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        if c == '\n' {
                            out.push('\n');
                        }
                        prev = c;
                    }
                }
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn build_aliases(opts: &CompilerOptions, _root: &Path) -> Vec<PathAlias> {
    let base = opts.base_url.as_deref().unwrap_or(".");

    let Some(paths) = &opts.paths else {
        return Vec::new();
    };

    let mut aliases = Vec::new();
    for (pattern, targets) in paths {
        // Pattern is like "@/*" or "~/*" — strip trailing *
        let prefix = pattern.trim_end_matches('*').to_string();
        let replacements: Vec<String> = targets
            .iter()
            .map(|t| {
                let stripped = t.trim_end_matches('*');
                let full = PathBuf::from(base).join(stripped);
                full.to_string_lossy().to_string()
            })
            .collect();
        aliases.push(PathAlias {
            prefix,
            replacements,
        });
    }

    // Sort by prefix length descending (most specific first)
    aliases.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
    aliases
}

// ---------------------------------------------------------------------------
// TypeScriptService
// ---------------------------------------------------------------------------

pub struct TypeScriptService {
    sqlite: SqliteStorage,
    aliases: Vec<PathAlias>,
}

impl TypeScriptService {
    pub fn new(sqlite: SqliteStorage, root: &Path) -> Self {
        let aliases = load_tsconfig_aliases(root);
        Self { sqlite, aliases }
    }
}

#[async_trait::async_trait]
impl LanguageService for TypeScriptService {
    fn name(&self) -> &str {
        "typescript"
    }

    fn is_applicable(&self, root: &Path) -> bool {
        // Has tsconfig.json or package.json with typescript
        if root.join("tsconfig.json").exists() || root.join("tsconfig.app.json").exists() {
            return true;
        }
        let pkg = root.join("package.json");
        if pkg.exists() {
            if let Ok(content) = std::fs::read_to_string(&pkg) {
                return content.contains("\"typescript\"");
            }
        }
        false
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        vec![
            // --- Group 1: Type System ---
            ToolDefinition {
                name: "ts_inspect_type".to_string(),
                description: "Inspect a TypeScript type/interface/class definition: fields, methods, extends. Finds the definition in the given file or across the project index.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "symbol_name": {
                            "type": "string",
                            "description": "Name of the type, interface, class, or enum"
                        },
                        "file": {
                            "type": "string",
                            "description": "Optional: file where the type is defined (speeds up lookup)"
                        }
                    },
                    "required": ["symbol_name"]
                }),
            },
            ToolDefinition {
                name: "ts_get_signature".to_string(),
                description: "Get the full signature of a TypeScript function or method: parameters, generics, return type.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "function_name": {
                            "type": "string",
                            "description": "Name of the function or method"
                        },
                        "file": {
                            "type": "string",
                            "description": "Optional: file to search in"
                        }
                    },
                    "required": ["function_name"]
                }),
            },
            ToolDefinition {
                name: "ts_get_exports".to_string(),
                description: "List all exported symbols from a TypeScript/JavaScript file (functions, types, constants, classes).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Relative path to the .ts/.tsx/.js file"
                        }
                    },
                    "required": ["file"]
                }),
            },
            // --- Group 2: Module Resolution ---
            ToolDefinition {
                name: "ts_resolve_import".to_string(),
                description: "Resolve a TypeScript import path to the actual file on disk. Handles tsconfig.json path aliases (@/, ~/), relative paths, and index files.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "import_path": {
                            "type": "string",
                            "description": "The import specifier (e.g. '@/utils/date', './Button', 'lodash')"
                        },
                        "from_file": {
                            "type": "string",
                            "description": "The file containing the import (for relative resolution)"
                        }
                    },
                    "required": ["import_path", "from_file"]
                }),
            },
            // --- Group 3: Safety ---
            ToolDefinition {
                name: "ts_check_file".to_string(),
                description: "Run `tsc --noEmit` and return type-checking diagnostics. Optionally filter to a specific file.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Optional: filter errors to this file"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "ts_find_references".to_string(),
                description: "Find all usages and imports of a symbol across the project (from the index).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "symbol_name": {
                            "type": "string",
                            "description": "Name of the symbol to search references for"
                        }
                    },
                    "required": ["symbol_name"]
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
            "ts_inspect_type" => self.tool_inspect_type(args, root).await,
            "ts_get_signature" => self.tool_get_signature(args, root).await,
            "ts_get_exports" => self.tool_get_exports(args, root).await,
            "ts_resolve_import" => self.tool_resolve_import(args, root).await,
            "ts_check_file" => self.tool_check_file(args, root).await,
            "ts_find_references" => self.tool_find_references(args).await,
            _ => Err(anyhow::anyhow!("Unknown TypeScript tool: {}", name)),
        }
    }
}

// ---------------------------------------------------------------------------
// Import resolution
// ---------------------------------------------------------------------------

static TS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "d.ts", "vue"];

fn resolve_import_path(
    import_path: &str,
    from_file: &Path,
    root: &Path,
    aliases: &[PathAlias],
) -> Option<PathBuf> {
    if import_path.starts_with('.') {
        // Relative import
        let dir = from_file.parent().unwrap_or(root);
        let candidate = dir.join(import_path);
        return try_resolve_file(root, &candidate);
    }

    // Try path aliases
    for alias in aliases {
        if import_path.starts_with(&alias.prefix) {
            let rest = &import_path[alias.prefix.len()..];
            for replacement in &alias.replacements {
                let candidate = root.join(replacement).join(rest);
                if let Some(resolved) = try_resolve_file(root, &candidate) {
                    return Some(resolved);
                }
            }
        }
    }

    // node_modules (best-effort)
    let nm = root.join("node_modules").join(import_path);
    if nm.is_dir() {
        // Try package.json main/types
        let pkg = nm.join("package.json");
        if pkg.exists() {
            if let Ok(raw) = std::fs::read_to_string(&pkg) {
                if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                    for key in ["types", "typings", "main", "module"] {
                        if let Some(entry) = v.get(key).and_then(|v| v.as_str()) {
                            let resolved = nm.join(entry);
                            if resolved.exists() {
                                return Some(resolved);
                            }
                        }
                    }
                }
            }
        }
        if let Some(resolved) = try_resolve_file(root, &nm.join("index")) {
            return Some(resolved);
        }
    }

    None
}

fn try_resolve_file(_root: &Path, candidate: &Path) -> Option<PathBuf> {
    // Direct match
    if candidate.is_file() {
        return Some(candidate.to_path_buf());
    }

    // Try extensions
    for ext in TS_EXTENSIONS {
        let with_ext = if *ext == "d.ts" {
            candidate.with_extension("d.ts")
        } else {
            candidate.with_extension(ext)
        };
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }

    // Try /index.ts etc.
    if candidate.is_dir() || !candidate.exists() {
        let dir = candidate.to_path_buf();
        for ext in &["ts", "tsx", "js", "jsx"] {
            let index = dir.join(format!("index.{}", ext));
            if index.is_file() {
                return Some(index);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tree-sitter helpers for TS
// ---------------------------------------------------------------------------

fn parse_ts_tree(code: &str) -> Option<tree_sitter::Tree> {
    crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&crate::indexer::parser::LANG_MANAGER.get_language("typescript").expect("Lang not loaded").language).ok()?;
        parser.parse(code, None)
    })
}

/// Extract the full text of a type/interface/class/enum definition by name
fn find_type_definition(code: &str, name: &str) -> Option<(String, String, bool)> {
    let tree = parse_ts_tree(code)?;
    let root = tree.root_node();
    find_type_in_node(root, code, name)
}

fn find_type_in_node(
    node: tree_sitter::Node<'_>,
    code: &str,
    name: &str,
) -> Option<(String, String, bool)> {
    // kind -> name child field
    let type_kinds = [
        "interface_declaration",
        "type_alias_declaration",
        "class_declaration",
        "enum_declaration",
    ];

    let kind = node.kind();

    if type_kinds.contains(&kind) {
        // Get the name node
        let name_node = node.child_by_field_name("name");
        if let Some(nn) = name_node {
            let node_name = &code[nn.byte_range()];
            if node_name == name {
                let text = code[node.byte_range()].to_string();

                // Check if exported
                let exported = if let Some(parent) = node.parent() {
                    parent.kind() == "export_statement"
                } else {
                    false
                };

                // Check for export_statement wrapping
                let full_text = if let Some(parent) = node.parent() {
                    if parent.kind() == "export_statement" {
                        code[parent.byte_range()].to_string()
                    } else {
                        text
                    }
                } else {
                    text
                };

                return Some((kind.to_string(), full_text, exported));
            }
        }
    }

    // Recurse
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(result) = find_type_in_node(child, code, name) {
            return Some(result);
        }
    }

    None
}

/// Find a function/method and return its full signature
fn find_function_signature(code: &str, name: &str) -> Option<(String, String, bool)> {
    let tree = parse_ts_tree(code)?;
    let root = tree.root_node();
    find_function_in_node(root, code, name)
}

fn find_function_in_node(
    node: tree_sitter::Node<'_>,
    code: &str,
    name: &str,
) -> Option<(String, String, bool)> {
    let kind = node.kind();

    match kind {
        "function_declaration" | "method_definition" => {
            let name_node = node.child_by_field_name("name");
            if let Some(nn) = name_node {
                let node_name = &code[nn.byte_range()];
                if node_name == name {
                    // Extract just the signature (without body)
                    let body_node = node.child_by_field_name("body");
                    let sig_end = body_node.map(|b| b.start_byte()).unwrap_or(node.end_byte());
                    let signature = code[node.start_byte()..sig_end].trim().to_string();

                    let exported = node
                        .parent()
                        .map(|p| p.kind() == "export_statement")
                        .unwrap_or(false);

                    return Some((kind.to_string(), signature, exported));
                }
            }
        }
        // Arrow function assigned to variable: const foo = (...) => ...
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    let decl_name = child.child_by_field_name("name");
                    if let Some(nn) = decl_name {
                        let node_name = &code[nn.byte_range()];
                        if node_name == name {
                            let value = child.child_by_field_name("value");
                            if let Some(val) = value {
                                if val.kind() == "arrow_function" || val.kind() == "function" {
                                    // Get everything up to the body
                                    let body = val.child_by_field_name("body");
                                    let sig_end =
                                        body.map(|b| b.start_byte()).unwrap_or(val.end_byte());
                                    let full_sig =
                                        code[node.start_byte()..sig_end].trim().to_string();

                                    let exported = node
                                        .parent()
                                        .map(|p| p.kind() == "export_statement")
                                        .unwrap_or(false);

                                    return Some(("arrow_function".to_string(), full_sig, exported));
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(result) = find_function_in_node(child, code, name) {
            return Some(result);
        }
    }

    None
}

/// Collect exported symbols from a file
fn collect_exports(code: &str) -> Vec<(String, String, u32)> {
    let Some(tree) = parse_ts_tree(code) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut exports = Vec::new();
    collect_exports_from_node(root, code, &mut exports);
    exports
}

fn collect_exports_from_node(
    node: tree_sitter::Node<'_>,
    code: &str,
    exports: &mut Vec<(String, String, u32)>,
) {
    if node.kind() == "export_statement" {
        // Find the declaration inside
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            match kind {
                "function_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration" => {
                    if let Some(nn) = child.child_by_field_name("name") {
                        let name = code[nn.byte_range()].to_string();
                        let label = match kind {
                            "function_declaration" => "function",
                            "class_declaration" => "class",
                            "interface_declaration" => "interface",
                            "type_alias_declaration" => "type",
                            "enum_declaration" => "enum",
                            _ => "unknown",
                        };
                        exports.push((
                            name,
                            label.to_string(),
                            child.start_position().row as u32 + 1,
                        ));
                    }
                }
                "lexical_declaration" | "variable_declaration" => {
                    let mut inner_cursor = child.walk();
                    for decl in child.children(&mut inner_cursor) {
                        if decl.kind() == "variable_declarator" {
                            if let Some(nn) = decl.child_by_field_name("name") {
                                let name = code[nn.byte_range()].to_string();
                                // Detect if it's an arrow function or regular value
                                let label = decl
                                    .child_by_field_name("value")
                                    .map(|v| {
                                        if v.kind() == "arrow_function" || v.kind() == "function" {
                                            "function"
                                        } else {
                                            "const"
                                        }
                                    })
                                    .unwrap_or("const");
                                exports.push((
                                    name,
                                    label.to_string(),
                                    decl.start_position().row as u32 + 1,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Named exports: export { Foo, Bar }
        let mut cursor2 = node.walk();
        for child in node.children(&mut cursor2) {
            if child.kind() == "export_clause" {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        if let Some(nn) = spec.child_by_field_name("name") {
                            let name = code[nn.byte_range()].to_string();
                            exports.push((
                                name,
                                "re-export".to_string(),
                                spec.start_position().row as u32 + 1,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Default export: export default ...
    // Handled by export_statement above

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "export_statement" {
            collect_exports_from_node(child, code, exports);
        }
    }
}

// ---------------------------------------------------------------------------
// File walker (reuse from vue module concept but inline here)
// ---------------------------------------------------------------------------

fn walk_ts_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_ts_recursive(root, &mut files);
    files
}

fn walk_ts_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.')
                || name == "node_modules"
                || name == "dist"
                || name == "build"
                || name == ".next"
            {
                continue;
            }
            walk_ts_recursive(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ["ts", "tsx", "js", "jsx"].contains(&ext) {
                out.push(path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

impl TypeScriptService {
    /// `ts_inspect_type` — find and display a type/interface/class definition
    async fn tool_inspect_type(&self, args: Value, root: &Path) -> Result<String> {
        let symbol_name = args
            .get("symbol_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'symbol_name' is required"))?;
        let file_path = args.get("file").and_then(|v| v.as_str());

        let mut out = format!("# Type: `{}`\n\n", symbol_name);

        // If file specified, search there first
        if let Some(fp) = file_path {
            let abs = root.join(fp);
            if abs.exists() {
                let code = tokio::fs::read_to_string(&abs).await?;
                if let Some((kind, text, exported)) = find_type_definition(&code, symbol_name) {
                    let exp = if exported { " (exported)" } else { "" };
                    out.push_str(&format!("**Kind:** {}{}\n", kind.replace('_', " "), exp));
                    out.push_str(&format!("**File:** `{}`\n\n", fp));
                    out.push_str(&format!("```typescript\n{}\n```\n", text));
                    return Ok(out);
                }
            }
        }

        // Search in project index
        let symbols = self.sqlite.get_symbol_by_name(symbol_name).await?;
        let type_syms: Vec<_> = symbols
            .iter()
            .filter(|s| {
                s.kind == crate::models::chunk::SymbolKind::Interface
                    || s.kind == crate::models::chunk::SymbolKind::Class
                    || s.kind == crate::models::chunk::SymbolKind::Type
                    || s.kind == crate::models::chunk::SymbolKind::Enum
            })
            .collect();

        if type_syms.is_empty() {
            // Fallback: scan TS files
            out.push_str("*Not found in index. Scanning project files...*\n\n");
            let ts_files = walk_ts_files(root);
            for f in &ts_files {
                let Ok(code) = tokio::fs::read_to_string(f).await else {
                    continue;
                };
                if let Some((kind, text, exported)) = find_type_definition(&code, symbol_name) {
                    let rel = f.strip_prefix(root).unwrap_or(f).display().to_string();
                    let exp = if exported { " (exported)" } else { "" };
                    out.push_str(&format!("**Kind:** {}{}\n", kind.replace('_', " "), exp));
                    out.push_str(&format!("**File:** `{}`\n\n", rel));
                    out.push_str(&format!("```typescript\n{}\n```\n", text));
                    return Ok(out);
                }
            }
            out.push_str("Type not found in project.\n");
        } else {
            for sym in &type_syms {
                let file = self.sqlite.get_file_by_id(sym.file_id).await?;
                let path = file.map(|f| f.path).unwrap_or_else(|| "?".to_string());

                out.push_str(&format!("**Kind:** {}\n", sym.kind));
                out.push_str(&format!("**Location:** `{}:{}`\n\n", path, sym.line_start));

                if let Some(ref sig) = sym.signature {
                    out.push_str(&format!("```typescript\n{}\n```\n\n", sig));
                }

                // Try to read full definition from file
                let abs = root.join(&path);
                if abs.exists() {
                    let code = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                    if let Some((_kind, text, exported)) = find_type_definition(&code, symbol_name)
                    {
                        let exp = if exported { " // exported" } else { "" };
                        out.push_str(&format!(
                            "**Full definition:**{}\n\n```typescript\n{}\n```\n\n",
                            exp, text
                        ));
                    }
                }
            }
        }

        Ok(out)
    }

    /// `ts_get_signature` — function/method signature
    async fn tool_get_signature(&self, args: Value, root: &Path) -> Result<String> {
        let function_name = args
            .get("function_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'function_name' is required"))?;
        let file_path = args.get("file").and_then(|v| v.as_str());

        let mut out = format!("# Function: `{}`\n\n", function_name);

        // Search in specific file
        if let Some(fp) = file_path {
            let abs = root.join(fp);
            if abs.exists() {
                let code = tokio::fs::read_to_string(&abs).await?;
                if let Some((kind, sig, exported)) = find_function_signature(&code, function_name) {
                    let exp = if exported { " (exported)" } else { "" };
                    out.push_str(&format!("**Kind:** {}{}\n", kind.replace('_', " "), exp));
                    out.push_str(&format!("**File:** `{}`\n\n", fp));
                    out.push_str(&format!("```typescript\n{}\n```\n", sig));
                    return Ok(out);
                }
            }
        }

        // Search in index
        let symbols = self.sqlite.get_symbol_by_name(function_name).await?;
        let fn_syms: Vec<_> = symbols
            .iter()
            .filter(|s| {
                s.kind == crate::models::chunk::SymbolKind::Function
                    || s.kind == crate::models::chunk::SymbolKind::Method
            })
            .collect();

        if fn_syms.is_empty() {
            // Scan files
            let ts_files = walk_ts_files(root);
            for f in &ts_files {
                let Ok(code) = tokio::fs::read_to_string(f).await else {
                    continue;
                };
                if let Some((kind, sig, exported)) = find_function_signature(&code, function_name) {
                    let rel = f.strip_prefix(root).unwrap_or(f).display().to_string();
                    let exp = if exported { " (exported)" } else { "" };
                    out.push_str(&format!("**Kind:** {}{}\n", kind.replace('_', " "), exp));
                    out.push_str(&format!("**File:** `{}`\n\n", rel));
                    out.push_str(&format!("```typescript\n{}\n```\n", sig));
                    return Ok(out);
                }
            }
            out.push_str("Function not found in project.\n");
        } else {
            for sym in &fn_syms {
                let file = self.sqlite.get_file_by_id(sym.file_id).await?;
                let path = file.map(|f| f.path).unwrap_or_else(|| "?".to_string());

                out.push_str(&format!("**Location:** `{}:{}`\n", path, sym.line_start));

                if let Some(ref sig) = sym.signature {
                    out.push_str(&format!("```typescript\n{}\n```\n\n", sig));
                } else {
                    // Try reading from file
                    let abs = root.join(&path);
                    if abs.exists() {
                        let code = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                        if let Some((_kind, sig, _exported)) =
                            find_function_signature(&code, function_name)
                        {
                            out.push_str(&format!("```typescript\n{}\n```\n\n", sig));
                        }
                    }
                }
            }
        }

        Ok(out)
    }

    /// `ts_get_exports` — list all exports from a file
    async fn tool_get_exports(&self, args: Value, root: &Path) -> Result<String> {
        let file_path = args
            .get("file")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'file' is required"))?;

        let abs = root.join(file_path);
        if !abs.exists() {
            return Err(anyhow::anyhow!("File not found: {}", file_path));
        }

        let code = tokio::fs::read_to_string(&abs).await?;
        let exports = collect_exports(&code);

        let mut out = format!("# Exports: `{}`\n\n", file_path);

        if exports.is_empty() {
            out.push_str("No exports found.\n");
        } else {
            out.push_str("| Symbol | Kind | Line |\n");
            out.push_str("|--------|------|------|\n");
            for (name, kind, line) in &exports {
                out.push_str(&format!("| `{}` | {} | {} |\n", name, kind, line));
            }
        }

        Ok(out)
    }

    /// `ts_resolve_import` — resolve import path to file on disk
    async fn tool_resolve_import(&self, args: Value, root: &Path) -> Result<String> {
        let import_path = args
            .get("import_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'import_path' is required"))?;
        let from_file = args
            .get("from_file")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'from_file' is required"))?;

        let abs_from = root.join(from_file);
        let mut out = format!("# Resolve: `{}`\n\n", import_path);
        out.push_str(&format!("**From:** `{}`\n\n", from_file));

        // Show configured aliases
        if !self.aliases.is_empty() {
            out.push_str("**Active aliases:**\n");
            for a in &self.aliases {
                out.push_str(&format!(
                    "- `{}*` -> `{}`\n",
                    a.prefix,
                    a.replacements.join(", ")
                ));
            }
            out.push('\n');
        }

        match resolve_import_path(import_path, &abs_from, root, &self.aliases) {
            Some(resolved) => {
                let rel = resolved
                    .strip_prefix(root)
                    .unwrap_or(&resolved)
                    .display()
                    .to_string();
                out.push_str(&format!("**Resolved to:** `{}`\n", rel));

                // Show what exports are available
                if resolved.exists() {
                    let ext = resolved.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ["ts", "tsx", "js", "jsx"].contains(&ext) {
                        let code = tokio::fs::read_to_string(&resolved)
                            .await
                            .unwrap_or_default();
                        let exports = collect_exports(&code);
                        if !exports.is_empty() {
                            out.push_str("\n**Available exports:**\n");
                            for (name, kind, _) in &exports {
                                out.push_str(&format!("- `{}` ({})\n", name, kind));
                            }
                        }
                    }
                }
            }
            None => {
                out.push_str("**Could not resolve.**\n\n");
                out.push_str("Attempted:\n");
                if import_path.starts_with('.') {
                    let dir = abs_from.parent().unwrap_or(root);
                    out.push_str(&format!(
                        "- Relative from `{}`\n",
                        dir.strip_prefix(root).unwrap_or(dir).display()
                    ));
                } else {
                    for a in &self.aliases {
                        if import_path.starts_with(&a.prefix) {
                            out.push_str(&format!(
                                "- Alias `{}` -> `{}`\n",
                                a.prefix,
                                a.replacements.join(", ")
                            ));
                        }
                    }
                }
                out.push_str("- Extensions: .ts, .tsx, .js, .jsx, .d.ts, .vue\n");
                out.push_str("- Index files: index.ts, index.tsx, index.js, index.jsx\n");
            }
        }

        Ok(out)
    }

    /// `ts_check_file` — run tsc --noEmit
    async fn tool_check_file(&self, args: Value, root: &Path) -> Result<String> {
        let file_filter = args.get("file").and_then(|v| v.as_str());

        // Determine tsc command: npx tsc or ./node_modules/.bin/tsc
        let tsc_path = root.join("node_modules/.bin/tsc");
        let (cmd_name, cmd_args) = if tsc_path.exists() {
            (
                tsc_path.to_string_lossy().to_string(),
                vec!["--noEmit", "--pretty", "false"],
            )
        } else {
            (
                "npx".to_string(),
                vec!["tsc", "--noEmit", "--pretty", "false"],
            )
        };

        let output = tokio::process::Command::new(&cmd_name)
            .args(&cmd_args)
            .current_dir(root)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        // Parse TSC output: src/file.ts(10,5): error TS2322: ...
        let mut diagnostics: Vec<(String, u32, String, String, String)> = Vec::new();
        for line in combined.lines() {
            if let Some(cap) = TSC_DIAGNOSTIC_RE.captures(line) {
                let file = cap[1].to_string();
                let line_num: u32 = cap[2].parse().unwrap_or(0);
                let level = cap[4].to_string();
                let code = cap[5].to_string();
                let msg = cap[6].to_string();

                if let Some(filter) = file_filter {
                    if !file.contains(filter) && !file.ends_with(filter) {
                        continue;
                    }
                }

                diagnostics.push((file, line_num, level, code, msg));
            }
        }

        let mut out = String::from("# TypeScript Check\n\n");

        if diagnostics.is_empty() {
            if output.status.success() {
                out.push_str("No type errors found.\n");
            } else {
                // tsc failed but we couldn't parse output
                let truncated: String = combined.chars().take(2000).collect();
                out.push_str("tsc returned errors but output couldn't be parsed:\n\n");
                out.push_str(&format!("```\n{}\n```\n", truncated.trim()));
            }
        } else {
            let errors = diagnostics.iter().filter(|d| d.2 == "error").count();
            let warnings = diagnostics.iter().filter(|d| d.2 == "warning").count();
            out.push_str(&format!(
                "Found **{}** error(s), **{}** warning(s)\n\n",
                errors, warnings
            ));

            for (file, line, level, code, msg) in &diagnostics {
                out.push_str(&format!(
                    "- **[{}]** `{}:{}` ({}): {}\n",
                    level.to_uppercase(),
                    file,
                    line,
                    code,
                    msg
                ));
            }
        }

        Ok(out)
    }

    /// `ts_find_references` — find symbol usages from index
    async fn tool_find_references(&self, args: Value) -> Result<String> {
        let symbol_name = args
            .get("symbol_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'symbol_name' is required"))?;

        let refs = self.sqlite.get_incoming_references(symbol_name).await?;

        let mut out = format!("# References to `{}`\n\n", symbol_name);

        if refs.is_empty() {
            out.push_str("No references found in the project index.\n\n");
            out.push_str("*Tip: run `gofer index sync` to update the index.*\n");
        } else {
            out.push_str(&format!("Found **{}** reference(s):\n\n", refs.len()));

            for r in &refs {
                // Get source symbol info
                let source = self.sqlite.get_symbol_by_id(r.source_symbol_id).await?;
                if let Some(src) = source {
                    let file = self.sqlite.get_file_by_id(src.file_id).await?;
                    let path = file.map(|f| f.path).unwrap_or_else(|| "?".to_string());
                    out.push_str(&format!(
                        "- `{}:{}` in `{}` ({})\n",
                        path, r.line, src.name, r.kind
                    ));
                } else {
                    out.push_str(&format!(
                        "- line {} ({}) [source symbol not found]\n",
                        r.line, r.kind
                    ));
                }
            }
        }

        Ok(out)
    }
}
