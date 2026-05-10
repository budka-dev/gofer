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

/// Walk up from the requested file to find the closest tsconfig.json that
/// includes it. Falls back to any tsconfig found along the way.
fn find_tsconfig_for(root: &Path, rel_file: &str) -> Option<PathBuf> {
    let abs_file = root.join(rel_file);
    let mut cur = abs_file.parent()?;
    loop {
        for name in &["tsconfig.json", "tsconfig.app.json", "jsconfig.json"] {
            let candidate = cur.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if cur == root {
            return None;
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return None,
        }
    }
}

/// Discover all tsconfig.json files reachable from the project root, including
/// those inside Bun/pnpm/yarn workspaces declared in the root package.json.
pub(crate) fn discover_workspace_tsconfigs(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    // 1. Root tsconfig — common case
    for name in &["tsconfig.json", "tsconfig.app.json"] {
        let p = root.join(name);
        if p.exists() {
            out.push(p);
        }
    }

    // 2. package.json workspaces — handles Bun, pnpm (when "workspaces" is mirrored
    //    into package.json), yarn classic
    let pkg_path = root.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&pkg_path) {
        if let Ok(pkg) = serde_json::from_str::<Value>(&raw) {
            let patterns = match pkg.get("workspaces") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>(),
                Some(Value::Object(obj)) => obj
                    .get("packages")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            for pat in patterns {
                expand_workspace_pattern(root, &pat, &mut out);
            }
        }
    }

    // 3. pnpm-workspace.yaml — best-effort plain text scan to keep deps thin
    let pnpm_ws = root.join("pnpm-workspace.yaml");
    if let Ok(raw) = std::fs::read_to_string(&pnpm_ws) {
        for line in raw.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("- \""))
                .or_else(|| trimmed.strip_prefix("- '"))
            {
                let pat = rest.trim_end_matches(['"', '\'']).to_string();
                if !pat.is_empty() {
                    expand_workspace_pattern(root, &pat, &mut out);
                }
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

fn expand_workspace_pattern(root: &Path, pattern: &str, out: &mut Vec<PathBuf>) {
    // Only expand simple `dir/*` and literal `dir/sub` patterns. Anything more
    // exotic (recursive globs, negations) we ignore — adding a glob crate just
    // for tsconfig discovery isn't worth the dependency weight.
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let dir = root.join(prefix);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    push_tsconfigs_in(&entry.path(), out);
                }
            }
        }
    } else if !pattern.contains('*') {
        push_tsconfigs_in(&root.join(pattern), out);
    }
}

fn push_tsconfigs_in(dir: &Path, out: &mut Vec<PathBuf>) {
    for name in &["tsconfig.json", "tsconfig.app.json"] {
        let p = dir.join(name);
        if p.exists() {
            out.push(p);
        }
    }
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

/// Public no-aliases entry point for callers outside this module (used by
/// `context_bundle` to resolve `@`-aliased and workspace imports).
pub(crate) fn resolve_import_path_public(
    import_path: &str,
    from_file: &Path,
    root: &Path,
) -> Option<PathBuf> {
    resolve_import_path(import_path, from_file, root, &[])
}

fn resolve_import_path(
    import_path: &str,
    from_file: &Path,
    root: &Path,
    fallback_aliases: &[PathAlias],
) -> Option<PathBuf> {
    if import_path.starts_with('.') {
        // Relative import
        let dir = from_file.parent().unwrap_or(root);
        let candidate = dir.join(import_path);
        return try_resolve_file(root, &candidate);
    }

    // Walk up from `from_file` to find the closest tsconfig.json — its `paths`
    // win over the root tsconfig's. This is what makes monorepo aliases like
    // `@slave/contracts → ../slave-core-contracts/src/index.ts` actually resolve
    // when `from_file` lives in `slave-core-be/`.
    let local_aliases = nearest_tsconfig_aliases(from_file, root);

    for set in [&local_aliases, fallback_aliases] {
        for alias in set.iter() {
            // Match the longest of {exact prefix, prefix-with-trailing-slash}.
            // tsconfig "paths": { "@slave/contracts": ["..."], "@slave/contracts/*": ["..."] }
            // both need to map; the prefix here may already end with "/".
            let stripped_prefix = alias.prefix.trim_end_matches('/');
            let matches_exact = import_path == stripped_prefix;
            let matches_subpath = !alias.prefix.is_empty()
                && (import_path.starts_with(&alias.prefix)
                    || import_path.starts_with(&format!("{}/", stripped_prefix)));
            if !matches_exact && !matches_subpath {
                continue;
            }

            let rest: &str = if matches_exact {
                ""
            } else if let Some(r) = import_path.strip_prefix(&alias.prefix) {
                r
            } else {
                import_path
                    .strip_prefix(&format!("{}/", stripped_prefix))
                    .unwrap_or("")
            };

            for replacement in &alias.replacements {
                let candidate_str = if rest.is_empty() {
                    replacement.clone()
                } else if replacement.ends_with('/') || replacement.is_empty() {
                    format!("{}{}", replacement, rest)
                } else {
                    format!("{}/{}", replacement, rest)
                };
                let candidate = if Path::new(&candidate_str).is_absolute() {
                    PathBuf::from(&candidate_str)
                } else {
                    root.join(&candidate_str)
                };
                if let Some(resolved) = try_resolve_file(root, &candidate) {
                    return Some(resolved);
                }
            }
        }
    }

    // Workspace package.json — Bun/pnpm/yarn workspaces expose packages by their
    // declared `name`. Look up the workspace whose package.json `name` matches
    // the import's package portion and use its `exports` / `main` / `types`.
    if let Some(resolved) = resolve_workspace_package(import_path, root) {
        return Some(resolved);
    }

    // node_modules (best-effort)
    let nm = root.join("node_modules").join(import_path);
    if nm.is_dir() {
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

/// Find the closest tsconfig.json above `from_file` and load its aliases,
/// resolving `extends` chains. Returns aliases anchored at the tsconfig's
/// directory (so relative `paths` resolve correctly even outside the project root).
fn nearest_tsconfig_aliases(from_file: &Path, root: &Path) -> Vec<PathAlias> {
    let mut cur = from_file.parent();
    while let Some(dir) = cur {
        for name in &["tsconfig.json", "tsconfig.app.json", "jsconfig.json"] {
            let p = dir.join(name);
            if p.exists() {
                let aliases = load_aliases_from_tsconfig(&p);
                if !aliases.is_empty() {
                    return aliases;
                }
            }
        }
        if dir == root {
            break;
        }
        cur = dir.parent();
    }
    Vec::new()
}

/// Read a tsconfig and produce path aliases anchored at the tsconfig's directory.
/// Follows `extends` chains (one level deep is enough for almost all projects).
fn load_aliases_from_tsconfig(path: &Path) -> Vec<PathAlias> {
    fn read(path: &Path) -> Option<TsConfigFull> {
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&strip_json_comments(&raw)).ok()
    }

    let mut current = path.to_path_buf();
    let mut merged_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut merged_base: Option<String> = None;
    let mut visited: std::collections::HashSet<PathBuf> =
        std::collections::HashSet::new();

    loop {
        if !visited.insert(current.clone()) {
            break;
        }
        let cfg = match read(&current) {
            Some(c) => c,
            None => break,
        };

        if merged_base.is_none() {
            if let Some(base) = cfg.compiler_options.base_url {
                merged_base = Some(base);
            }
        }
        if let Some(paths) = cfg.compiler_options.paths {
            for (k, v) in paths {
                merged_paths.entry(k).or_insert(v);
            }
        }

        match cfg.extends {
            Some(ext) => {
                let parent = current.parent().unwrap_or(Path::new("."));
                current = if ext.starts_with('.') {
                    parent.join(&ext)
                } else {
                    // bare specifier — best-effort: try as file under parent
                    parent.join(&ext)
                };
                if !current.extension().map(|e| e == "json").unwrap_or(false) {
                    current = current.with_extension("json");
                }
            }
            None => break,
        }
    }

    if merged_paths.is_empty() {
        return Vec::new();
    }

    let cfg_dir = path.parent().unwrap_or(Path::new("."));
    let base = merged_base.as_deref().unwrap_or(".");
    let base_dir = cfg_dir.join(base);

    let mut aliases = Vec::new();
    for (pattern, targets) in merged_paths {
        let prefix = pattern.trim_end_matches('*').to_string();
        let replacements: Vec<String> = targets
            .iter()
            .map(|t| {
                let stripped = t.trim_end_matches('*');
                base_dir.join(stripped).to_string_lossy().to_string()
            })
            .collect();
        aliases.push(PathAlias {
            prefix,
            replacements,
        });
    }
    aliases.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
    aliases
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TsConfigFull {
    extends: Option<String>,
    #[serde(alias = "compilerOptions")]
    compiler_options: CompilerOptions,
}

/// If `import_path` matches the `name` field of a workspace package.json, return
/// the file that workspace exports for it (handling subpath imports too).
fn resolve_workspace_package(import_path: &str, root: &Path) -> Option<PathBuf> {
    let workspaces = collect_workspace_dirs(root);
    for ws in workspaces {
        let pkg_path = ws.join("package.json");
        let raw = std::fs::read_to_string(&pkg_path).ok()?;
        let pkg: Value = serde_json::from_str(&raw).ok()?;
        let name = pkg.get("name").and_then(|v| v.as_str())?;

        let subpath = if import_path == name {
            Some("")
        } else if let Some(rest) = import_path.strip_prefix(&format!("{}/", name)) {
            Some(rest)
        } else {
            None
        };
        let Some(subpath) = subpath else { continue };

        // 1. exports field — handle the common shapes only ("." / "./*" / map)
        if let Some(exports) = pkg.get("exports") {
            if let Some(p) = resolve_pkg_exports(exports, subpath, &ws) {
                return Some(p);
            }
        }

        // 2. Sub-path lookup: try to resolve `<ws>/src/<subpath>` and `<ws>/<subpath>`
        if !subpath.is_empty() {
            for base in &["src", ""] {
                let candidate = if base.is_empty() {
                    ws.join(subpath)
                } else {
                    ws.join(base).join(subpath)
                };
                if let Some(r) = try_resolve_file(root, &candidate) {
                    return Some(r);
                }
            }
        }

        // 3. main / module / types
        for key in ["types", "typings", "module", "main"] {
            if let Some(entry) = pkg.get(key).and_then(|v| v.as_str()) {
                let candidate = ws.join(entry);
                if candidate.is_file() {
                    return Some(candidate);
                }
                if let Some(r) = try_resolve_file(root, &candidate) {
                    return Some(r);
                }
            }
        }

        // 4. <ws>/src/index.ts fallback
        if let Some(r) = try_resolve_file(root, &ws.join("src").join("index")) {
            return Some(r);
        }
        if let Some(r) = try_resolve_file(root, &ws.join("index")) {
            return Some(r);
        }
    }
    None
}

fn resolve_pkg_exports(exports: &Value, subpath: &str, ws: &Path) -> Option<PathBuf> {
    // String shape: "exports": "./index.ts"
    if let Some(s) = exports.as_str() {
        if subpath.is_empty() {
            return Some(ws.join(s.trim_start_matches("./")));
        }
        return None;
    }

    // Object shape: { ".": ..., "./auth": ..., "./*": ... }
    let obj = exports.as_object()?;
    let key = if subpath.is_empty() {
        ".".to_string()
    } else {
        format!("./{}", subpath)
    };

    let entry = obj.get(&key).or_else(|| {
        // Try wildcard match — find a key like "./*" or "./feat/*"
        obj.iter().find_map(|(k, v)| {
            if let Some(prefix) = k.strip_suffix('*') {
                let prefix = prefix.trim_start_matches("./");
                if subpath.starts_with(prefix) {
                    let rest = &subpath[prefix.len()..];
                    return Some((v, rest));
                }
            }
            None
        }).map(|(v, _)| v)
    });

    let entry = entry?;
    let target_str = match entry {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("import")
            .or_else(|| o.get("require"))
            .or_else(|| o.get("default"))
            .and_then(|v| v.as_str())
            .map(String::from)?,
        _ => return None,
    };

    Some(ws.join(target_str.trim_start_matches("./")))
}

fn collect_workspace_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let pkg_path = root.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&pkg_path) {
        if let Ok(pkg) = serde_json::from_str::<Value>(&raw) {
            let patterns = match pkg.get("workspaces") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>(),
                Some(Value::Object(obj)) => obj
                    .get("packages")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            for pat in patterns {
                expand_workspace_dirs(root, &pat, &mut out);
            }
        }
    }

    // pnpm-workspace.yaml fallback
    let pnpm = root.join("pnpm-workspace.yaml");
    if let Ok(raw) = std::fs::read_to_string(&pnpm) {
        for line in raw.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("- \""))
                .or_else(|| trimmed.strip_prefix("- '"))
            {
                let pat = rest.trim_end_matches(['"', '\'']).to_string();
                if !pat.is_empty() {
                    expand_workspace_dirs(root, &pat, &mut out);
                }
            }
        }
    }

    // Sibling repos one level above root — covers slave-ai's "../slave-core-contracts"
    // monorepo-on-disk shape where each repo is a peer directory under a parent.
    if let Some(parent) = root.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && p != root && p.join("package.json").is_file() {
                    out.push(p);
                }
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

fn expand_workspace_dirs(root: &Path, pattern: &str, out: &mut Vec<PathBuf>) {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let dir = root.join(prefix);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    out.push(entry.path());
                }
            }
        }
    } else if !pattern.contains('*') {
        out.push(root.join(pattern));
    }
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

                // Stored signatures pre-fix were sometimes truncated to the
                // first line ("function foo(" with no params/return type).
                // If the cached value looks incomplete, re-parse the file to
                // recover the full signature so the user isn't stuck with a
                // partial answer until a force_reindex.
                let stored_looks_truncated = sym
                    .signature
                    .as_deref()
                    .map(|s| {
                        let trimmed = s.trim_end();
                        trimmed.ends_with('(')
                            || trimmed.ends_with(',')
                            || (!trimmed.contains(')')
                                && (trimmed.starts_with("function ")
                                    || trimmed.starts_with("async function ")
                                    || trimmed.starts_with("export function ")
                                    || trimmed.starts_with("export async function ")))
                    })
                    .unwrap_or(true);

                let final_sig: Option<String> = if stored_looks_truncated {
                    let abs = root.join(&path);
                    let from_file = if abs.exists() {
                        let code = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                        find_function_signature(&code, function_name).map(|(_, s, _)| s)
                    } else {
                        None
                    };
                    from_file.or_else(|| sym.signature.clone())
                } else {
                    sym.signature.clone()
                };

                if let Some(sig) = final_sig {
                    out.push_str(&format!("```typescript\n{}\n```\n\n", sig));
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

    /// `ts_check_file` — run tsc --noEmit against the nearest tsconfig.json.
    ///
    /// Previously this just ran `tsc` in `root` with no `-p` arg, so on monorepos
    /// without a root tsconfig (slave-ai, most pnpm/bun workspaces) tsc had no
    /// inputs and printed its `--help` banner, which we then returned as the
    /// "diagnostics". Now we walk up from the requested file to find the closest
    /// tsconfig.json, or iterate workspace tsconfigs when no `file` is given.
    async fn tool_check_file(&self, args: Value, root: &Path) -> Result<String> {
        let file_filter = args.get("file").and_then(|v| v.as_str());
        let manifest_arg = args
            .get("manifest_path")
            .or_else(|| args.get("tsconfig"))
            .and_then(|v| v.as_str());

        let tsconfigs: Vec<PathBuf> = if let Some(p) = manifest_arg {
            vec![root.join(p)]
        } else if let Some(f) = file_filter {
            match find_tsconfig_for(root, f) {
                Some(p) => vec![p],
                None => {
                    return Ok(format!(
                        "# TypeScript Check\n\nNo tsconfig.json found walking up from `{}`. \
                         Pass `manifest_path` or run from a directory containing tsconfig.json.\n",
                        f
                    ));
                }
            }
        } else {
            discover_workspace_tsconfigs(root)
        };

        if tsconfigs.is_empty() {
            return Ok(
                "# TypeScript Check\n\nNo tsconfig.json discovered in this project.\n".to_string(),
            );
        }

        let tsc_path = root.join("node_modules/.bin/tsc");
        let tsc_exists = tsc_path.exists();

        let mut all_diagnostics: Vec<(String, u32, String, String, String)> = Vec::new();
        let mut ran_any = false;
        let mut errors_unparsed: Vec<(PathBuf, String)> = Vec::new();

        for tsconfig in &tsconfigs {
            let tsconfig_arg = tsconfig.to_string_lossy().to_string();
            let (cmd_name, cmd_args): (String, Vec<&str>) = if tsc_exists {
                (
                    tsc_path.to_string_lossy().to_string(),
                    vec!["--noEmit", "--pretty", "false", "-p", &tsconfig_arg],
                )
            } else {
                (
                    "npx".to_string(),
                    vec![
                        "--no-install",
                        "tsc",
                        "--noEmit",
                        "--pretty",
                        "false",
                        "-p",
                        &tsconfig_arg,
                    ],
                )
            };

            let output = match tokio::process::Command::new(&cmd_name)
                .args(&cmd_args)
                .current_dir(root)
                .output()
                .await
            {
                Ok(o) => o,
                Err(e) => {
                    errors_unparsed.push((tsconfig.clone(), format!("spawn failed: {}", e)));
                    continue;
                }
            };

            ran_any = true;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}\n{}", stdout, stderr);

            let mut parsed_any = false;
            for line in combined.lines() {
                if let Some(cap) = TSC_DIAGNOSTIC_RE.captures(line) {
                    parsed_any = true;
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

                    all_diagnostics.push((file, line_num, level, code, msg));
                }
            }

            if !output.status.success() && !parsed_any {
                let truncated: String = combined.chars().take(2000).collect();
                errors_unparsed.push((tsconfig.clone(), truncated.trim().to_string()));
            }
        }

        let mut out = String::from("# TypeScript Check\n\n");
        out.push_str(&format!(
            "Checked {} tsconfig(s): {}\n\n",
            tsconfigs.len(),
            tsconfigs
                .iter()
                .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));

        if all_diagnostics.is_empty() && errors_unparsed.is_empty() {
            if ran_any {
                out.push_str("No type errors found.\n");
            } else {
                out.push_str("tsc could not be invoked — no diagnostics produced.\n");
            }
        } else {
            let errors = all_diagnostics.iter().filter(|d| d.2 == "error").count();
            let warnings = all_diagnostics.iter().filter(|d| d.2 == "warning").count();
            out.push_str(&format!(
                "Found **{}** error(s), **{}** warning(s)\n\n",
                errors, warnings
            ));

            for (file, line, level, code, msg) in &all_diagnostics {
                out.push_str(&format!(
                    "- **[{}]** `{}:{}` ({}): {}\n",
                    level.to_uppercase(),
                    file,
                    line,
                    code,
                    msg
                ));
            }

            for (cfg, blob) in &errors_unparsed {
                out.push_str(&format!(
                    "\n## tsc errors for `{}` (unparseable)\n\n```\n{}\n```\n",
                    cfg.display(),
                    blob
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
