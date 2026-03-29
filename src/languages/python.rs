use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{LanguageService, ToolDefinition};
use crate::storage::SqliteStorage;

// ---------------------------------------------------------------------------
// Environment detection models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum PackageManager {
    Poetry,
    Pdm,
    Pep621,
    Pipenv,
    Pip,
    Unknown,
}

impl std::fmt::Display for PackageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageManager::Poetry => write!(f, "Poetry"),
            PackageManager::Pdm => write!(f, "PDM"),
            PackageManager::Pep621 => write!(f, "PEP 621 (pyproject.toml)"),
            PackageManager::Pipenv => write!(f, "Pipenv"),
            PackageManager::Pip => write!(f, "pip (requirements.txt)"),
            PackageManager::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone)]
struct PythonEnvironment {
    manager: PackageManager,
    manifest_path: Option<PathBuf>,
    has_venv: bool,
    venv_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// pyproject.toml models (for TOML parsing)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PyProject {
    project: Option<PyProjectProject>,
    tool: Option<PyProjectTool>,
    #[serde(alias = "build-system")]
    build_system: Option<BuildSystem>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PyProjectProject {
    name: Option<String>,
    version: Option<String>,
    dependencies: Option<Vec<String>>,
    #[serde(alias = "optional-dependencies")]
    optional_dependencies: Option<toml::value::Table>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PyProjectTool {
    poetry: Option<PoetrySection>,
    pdm: Option<toml::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PoetrySection {
    name: Option<String>,
    version: Option<String>,
    dependencies: Option<toml::value::Table>,
    #[serde(alias = "dev-dependencies")]
    dev_dependencies: Option<toml::value::Table>,
    group: Option<toml::value::Table>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct BuildSystem {
    requires: Option<Vec<String>>,
    #[serde(alias = "build-backend")]
    build_backend: Option<String>,
}

// ---------------------------------------------------------------------------
// PythonService
// ---------------------------------------------------------------------------

pub struct PythonService {
    #[allow(dead_code)]
    sqlite: SqliteStorage,
    env: PythonEnvironment,
}

impl PythonService {
    pub fn new(sqlite: SqliteStorage, root: &Path) -> Self {
        let env = detect_environment(root);
        Self { sqlite, env }
    }
}

// ---------------------------------------------------------------------------
// Environment detection
// ---------------------------------------------------------------------------

fn detect_environment(root: &Path) -> PythonEnvironment {
    // Detect venv
    let (has_venv, venv_path) = detect_venv(root);

    // Detect package manager & manifest
    let pyproject = root.join("pyproject.toml");
    let pipfile = root.join("Pipfile");
    let requirements = root.join("requirements.txt");

    if pyproject.exists() {
        if let Ok(raw) = std::fs::read_to_string(&pyproject) {
            if let Ok(parsed) = toml::from_str::<PyProject>(&raw) {
                // Check Poetry
                if parsed
                    .tool
                    .as_ref()
                    .and_then(|t| t.poetry.as_ref())
                    .is_some()
                {
                    return PythonEnvironment {
                        manager: PackageManager::Poetry,
                        manifest_path: Some(pyproject),
                        has_venv,
                        venv_path,
                    };
                }
                // Check PDM
                if parsed.tool.as_ref().and_then(|t| t.pdm.as_ref()).is_some() {
                    return PythonEnvironment {
                        manager: PackageManager::Pdm,
                        manifest_path: Some(pyproject),
                        has_venv,
                        venv_path,
                    };
                }
                // PEP 621 (has [project] section)
                if parsed.project.is_some() {
                    return PythonEnvironment {
                        manager: PackageManager::Pep621,
                        manifest_path: Some(pyproject),
                        has_venv,
                        venv_path,
                    };
                }
                // Has pyproject.toml but unclear manager — still report it
                return PythonEnvironment {
                    manager: PackageManager::Pep621,
                    manifest_path: Some(pyproject),
                    has_venv,
                    venv_path,
                };
            }
        }
    }

    if pipfile.exists() {
        return PythonEnvironment {
            manager: PackageManager::Pipenv,
            manifest_path: Some(pipfile),
            has_venv,
            venv_path,
        };
    }

    if requirements.exists() {
        return PythonEnvironment {
            manager: PackageManager::Pip,
            manifest_path: Some(requirements),
            has_venv,
            venv_path,
        };
    }

    PythonEnvironment {
        manager: PackageManager::Unknown,
        manifest_path: None,
        has_venv,
        venv_path,
    }
}

fn detect_venv(root: &Path) -> (bool, Option<PathBuf>) {
    for dir in &[".venv", "venv", ".env", "env"] {
        let candidate = root.join(dir);
        // A valid venv has a pyvenv.cfg or bin/python
        if candidate.is_dir()
            && (candidate.join("pyvenv.cfg").exists()
                || candidate.join("bin").join("python").exists()
                || candidate.join("Scripts").join("python.exe").exists())
        {
            return (true, Some(candidate));
        }
    }
    (false, None)
}

// ---------------------------------------------------------------------------
// Manifest reading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Dependency {
    name: String,
    version: String,
    group: String,
}

fn read_manifest(root: &Path, env: &PythonEnvironment) -> Result<(String, Vec<Dependency>)> {
    match env.manager {
        PackageManager::Poetry => read_poetry_manifest(root),
        PackageManager::Pdm | PackageManager::Pep621 => read_pep621_manifest(root),
        PackageManager::Pipenv => read_pipfile(root),
        PackageManager::Pip => read_requirements_txt(root),
        PackageManager::Unknown => Ok(("No manifest found".to_string(), Vec::new())),
    }
}

fn read_poetry_manifest(root: &Path) -> Result<(String, Vec<Dependency>)> {
    let raw = std::fs::read_to_string(root.join("pyproject.toml"))?;
    let parsed: PyProject = toml::from_str(&raw)?;

    let mut deps = Vec::new();
    let poetry = parsed.tool.as_ref().and_then(|t| t.poetry.as_ref());

    if let Some(poetry) = poetry {
        // Main dependencies
        if let Some(ref table) = poetry.dependencies {
            for (name, val) in table {
                if name == "python" {
                    continue;
                }
                let version = parse_poetry_version(val);
                deps.push(Dependency {
                    name: name.clone(),
                    version,
                    group: "main".to_string(),
                });
            }
        }
        // Dev dependencies (legacy format)
        if let Some(ref table) = poetry.dev_dependencies {
            for (name, val) in table {
                let version = parse_poetry_version(val);
                deps.push(Dependency {
                    name: name.clone(),
                    version,
                    group: "dev".to_string(),
                });
            }
        }
        // Dependency groups (Poetry 1.2+)
        if let Some(ref groups) = poetry.group {
            for (group_name, group_val) in groups {
                if let Some(group_deps) = group_val
                    .as_table()
                    .and_then(|t| t.get("dependencies"))
                    .and_then(|d| d.as_table())
                {
                    for (name, val) in group_deps {
                        let version = parse_poetry_version_from_toml(val);
                        deps.push(Dependency {
                            name: name.clone(),
                            version,
                            group: group_name.clone(),
                        });
                    }
                }
            }
        }
    }

    let project_name = poetry
        .and_then(|p| p.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    Ok((format!("Poetry project: {}", project_name), deps))
}

fn parse_poetry_version(val: &toml::Value) -> String {
    match val {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .to_string(),
        _ => "*".to_string(),
    }
}

fn parse_poetry_version_from_toml(val: &toml::Value) -> String {
    match val {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .to_string(),
        _ => "*".to_string(),
    }
}

fn read_pep621_manifest(root: &Path) -> Result<(String, Vec<Dependency>)> {
    let raw = std::fs::read_to_string(root.join("pyproject.toml"))?;
    let parsed: PyProject = toml::from_str(&raw)?;

    let mut deps = Vec::new();

    if let Some(ref proj) = parsed.project {
        // Main dependencies from [project.dependencies]
        if let Some(ref dep_list) = proj.dependencies {
            for spec in dep_list {
                let (name, version) = parse_pep508_spec(spec);
                deps.push(Dependency {
                    name,
                    version,
                    group: "main".to_string(),
                });
            }
        }
        // Optional dependencies
        if let Some(ref opt_deps) = proj.optional_dependencies {
            for (group_name, val) in opt_deps {
                if let Some(arr) = val.as_array() {
                    for item in arr {
                        if let Some(spec) = item.as_str() {
                            let (name, version) = parse_pep508_spec(spec);
                            deps.push(Dependency {
                                name,
                                version,
                                group: group_name.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    let project_name = parsed
        .project
        .as_ref()
        .and_then(|p| p.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    Ok((format!("PEP 621 project: {}", project_name), deps))
}

/// Parse a PEP 508 dependency specifier: "requests>=2.28,<3.0" -> ("requests", ">=2.28,<3.0")
fn parse_pep508_spec(spec: &str) -> (String, String) {
    let spec = spec.trim();
    // Find the first char that starts a version constraint or extras
    if let Some(pos) = spec.find(['>', '<', '=', '!', '~', '[', ';']) {
        let name = spec[..pos].trim().to_string();
        let rest = spec[pos..].trim().to_string();
        // Strip extras [xxx] from name if present
        let name = name.split('[').next().unwrap_or(&name).trim().to_string();
        (name, rest)
    } else {
        (spec.to_string(), "*".to_string())
    }
}

fn read_pipfile(root: &Path) -> Result<(String, Vec<Dependency>)> {
    let raw = std::fs::read_to_string(root.join("Pipfile"))?;
    let parsed: toml::Value = toml::from_str(&raw)?;

    let mut deps = Vec::new();

    if let Some(packages) = parsed.get("packages").and_then(|v| v.as_table()) {
        for (name, val) in packages {
            let version = match val {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string(),
                _ => "*".to_string(),
            };
            deps.push(Dependency {
                name: name.clone(),
                version,
                group: "main".to_string(),
            });
        }
    }

    if let Some(dev_packages) = parsed.get("dev-packages").and_then(|v| v.as_table()) {
        for (name, val) in dev_packages {
            let version = match val {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string(),
                _ => "*".to_string(),
            };
            deps.push(Dependency {
                name: name.clone(),
                version,
                group: "dev".to_string(),
            });
        }
    }

    Ok(("Pipenv project".to_string(), deps))
}

fn read_requirements_txt(root: &Path) -> Result<(String, Vec<Dependency>)> {
    // Try multiple requirements files
    let candidates = [
        "requirements.txt",
        "requirements-dev.txt",
        "requirements_dev.txt",
        "requirements/base.txt",
        "requirements/dev.txt",
    ];

    let mut deps = Vec::new();

    for candidate in &candidates {
        let path = root.join(candidate);
        if path.exists() {
            let group = if candidate.contains("dev") {
                "dev"
            } else {
                "main"
            };
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let line = line.trim();
                    // Skip comments, blank lines, -r includes, flags
                    if line.is_empty()
                        || line.starts_with('#')
                        || line.starts_with('-')
                        || line.starts_with("--")
                    {
                        continue;
                    }
                    let (name, version) = parse_pep508_spec(line);
                    if !name.is_empty() {
                        deps.push(Dependency {
                            name,
                            version,
                            group: group.to_string(),
                        });
                    }
                }
            }
        }
    }

    Ok(("pip project (requirements.txt)".to_string(), deps))
}

// ---------------------------------------------------------------------------
// Tree-sitter code inspection
// ---------------------------------------------------------------------------

struct ClassInfo {
    name: String,
    bases: Vec<String>,
    methods: Vec<MethodInfo>,
    fields: Vec<FieldInfo>,
    decorators: Vec<String>,
    line: u32,
}

struct MethodInfo {
    name: String,
    params: String,
    return_type: Option<String>,
    decorators: Vec<String>,
}

struct FieldInfo {
    name: String,
    type_annotation: Option<String>,
}

struct FunctionInfo {
    name: String,
    params: String,
    return_type: Option<String>,
    decorators: Vec<String>,
    line: u32,
}

fn inspect_python_code(code: &str) -> Result<(Vec<ClassInfo>, Vec<FunctionInfo>)> {
    let language = crate::indexer::parser::LANG_MANAGER.get_language("python").expect("Lang not loaded").language.clone();
    
    let tree = crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&language).map_err(|e| anyhow::anyhow!("Failed to set Python language: {}", e))?;
        parser.parse(code, None).ok_or_else(|| anyhow::anyhow!("Failed to parse Python code"))
    })?;

    let root = tree.root_node();
    let mut classes = Vec::new();
    let mut functions = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "decorated_definition" => {
                // Collect decorators, then process inner definition
                let decorators = collect_decorators(&child, code);
                if let Some(inner) = find_inner_definition(&child) {
                    match inner.kind() {
                        "class_definition" => {
                            let mut ci = parse_class_node(&inner, code);
                            ci.decorators = decorators;
                            classes.push(ci);
                        }
                        "function_definition" => {
                            let mut fi = parse_function_node(&inner, code);
                            fi.decorators = decorators;
                            functions.push(fi);
                        }
                        _ => {}
                    }
                }
            }
            "class_definition" => {
                classes.push(parse_class_node(&child, code));
            }
            "function_definition" => {
                functions.push(parse_function_node(&child, code));
            }
            _ => {}
        }
    }

    Ok((classes, functions))
}

fn collect_decorators(node: &tree_sitter::Node, code: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = node_text(&child, code).trim().to_string();
            // Strip the leading @
            let text = if let Some(stripped) = text.strip_prefix('@') {
                stripped.to_string()
            } else {
                text
            };
            decorators.push(text);
        }
    }
    decorators
}

fn find_inner_definition<'a>(
    decorated: &'a tree_sitter::Node<'a>,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = decorated.walk();
    for child in decorated.children(&mut cursor) {
        match child.kind() {
            "class_definition" | "function_definition" => return Some(child),
            _ => {}
        }
    }
    None
}

fn parse_class_node(node: &tree_sitter::Node, code: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, code).to_string())
        .unwrap_or_default();

    let line = node.start_position().row as u32 + 1;

    // Base classes from argument_list (superclasses)
    let bases = node
        .child_by_field_name("superclasses")
        .map(|args| {
            let mut bases = Vec::new();
            let mut cursor = args.walk();
            for child in args.children(&mut cursor) {
                let kind = child.kind();
                if kind != "(" && kind != ")" && kind != "," {
                    bases.push(node_text(&child, code).trim().to_string());
                }
            }
            bases
        })
        .unwrap_or_default();

    // Body: methods and class-level annotations
    let mut methods = Vec::new();
    let mut fields = Vec::new();

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    let fi = parse_function_node(&child, code);
                    methods.push(MethodInfo {
                        name: fi.name,
                        params: fi.params,
                        return_type: fi.return_type,
                        decorators: fi.decorators,
                    });
                }
                "decorated_definition" => {
                    let decorators = collect_decorators(&child, code);
                    if let Some(inner) = find_inner_definition(&child) {
                        if inner.kind() == "function_definition" {
                            let fi = parse_function_node(&inner, code);
                            methods.push(MethodInfo {
                                name: fi.name,
                                params: fi.params,
                                return_type: fi.return_type,
                                decorators,
                            });
                        }
                    }
                }
                "expression_statement" => {
                    // Type annotations like: name: str = "default"
                    if let Some(assign) = child.child(0) {
                        if assign.kind() == "type" || assign.kind() == "assignment" {
                            if let Some(field) = extract_field_annotation(&assign, code) {
                                fields.push(field);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    ClassInfo {
        name,
        bases,
        methods,
        fields,
        decorators: Vec::new(),
        line,
    }
}

fn extract_field_annotation(node: &tree_sitter::Node, code: &str) -> Option<FieldInfo> {
    // Handle: `name: type` and `name: type = value`
    if node.kind() == "type" {
        // This is a bare annotation: `name: type`
        let text = node_text(node, code);
        let parts: Vec<&str> = text.splitn(2, ':').collect();
        if parts.len() == 2 {
            return Some(FieldInfo {
                name: parts[0].trim().to_string(),
                type_annotation: Some(parts[1].trim().to_string()),
            });
        }
    }
    if node.kind() == "assignment" {
        // Could be annotated: `name: type = value`
        let text = node_text(node, code);
        if text.contains(':') {
            let parts: Vec<&str> = text.splitn(2, ':').collect();
            if parts.len() == 2 {
                let type_part = parts[1].split('=').next().unwrap_or("").trim();
                return Some(FieldInfo {
                    name: parts[0].trim().to_string(),
                    type_annotation: if type_part.is_empty() {
                        None
                    } else {
                        Some(type_part.to_string())
                    },
                });
            }
        }
    }
    None
}

fn parse_function_node(node: &tree_sitter::Node, code: &str) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, code).to_string())
        .unwrap_or_default();

    let line = node.start_position().row as u32 + 1;

    // Parameters
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(&n, code).to_string())
        .unwrap_or_else(|| "()".to_string());

    // Return type annotation
    let return_type = node.child_by_field_name("return_type").map(|n| {
        let text = node_text(&n, code).trim().to_string();
        // Strip leading "-> " if present
        if let Some(stripped) = text.strip_prefix("->") {
            stripped.trim().to_string()
        } else {
            text
        }
    });

    FunctionInfo {
        name,
        params,
        return_type,
        decorators: Vec::new(),
        line,
    }
}

fn node_text<'a>(node: &tree_sitter::Node, code: &'a str) -> &'a str {
    &code[node.byte_range()]
}

// ---------------------------------------------------------------------------
// Import resolution
// ---------------------------------------------------------------------------

fn resolve_python_import(
    import_path: &str,
    from_file: &Path,
    root: &Path,
) -> Option<(PathBuf, String)> {
    let import_path = import_path.trim();

    // Count leading dots for relative imports
    let dot_count = import_path.chars().take_while(|c| *c == '.').count();
    let module_part = &import_path[dot_count..];

    if dot_count > 0 {
        // Relative import
        let base_dir = if from_file.is_file() {
            from_file.parent().unwrap_or(root)
        } else {
            from_file
        };

        // Go up (dot_count - 1) directories (one dot = current package)
        let mut anchor = base_dir.to_path_buf();
        for _ in 1..dot_count {
            anchor = anchor.parent().unwrap_or(root).to_path_buf();
        }

        if module_part.is_empty() {
            // `from . import x` -> current package __init__.py
            let init = anchor.join("__init__.py");
            if init.exists() {
                return Some((init, "relative package".to_string()));
            }
            return None;
        }

        return resolve_module_parts(module_part, &anchor, "relative");
    }

    // Absolute import — check local project first
    if let Some(result) = resolve_module_parts(import_path, root, "project") {
        return Some(result);
    }

    // Check src/ layout
    let src = root.join("src");
    if src.is_dir() {
        if let Some(result) = resolve_module_parts(import_path, &src, "project (src/)") {
            return Some(result);
        }
    }

    // Check if it's a stdlib module
    if is_likely_stdlib(import_path.split('.').next().unwrap_or(import_path)) {
        return Some((PathBuf::from("<stdlib>"), "standard library".to_string()));
    }

    // Otherwise it's likely a third-party package
    Some((PathBuf::from("<site-packages>"), "third-party".to_string()))
}

fn resolve_module_parts(module: &str, base: &Path, kind: &str) -> Option<(PathBuf, String)> {
    let parts: Vec<&str> = module.split('.').collect();
    let mut current = base.to_path_buf();

    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;
        let as_file = current.join(format!("{}.py", part));
        let as_dir = current.join(part);

        if is_last {
            // Last part: prefer file, then package
            if as_file.exists() {
                return Some((as_file, kind.to_string()));
            }
            if as_dir.is_dir() && as_dir.join("__init__.py").exists() {
                return Some((as_dir.join("__init__.py"), format!("{} (package)", kind)));
            }
            return None;
        } else {
            // Intermediate parts must be packages
            if as_dir.is_dir() {
                current = as_dir;
            } else {
                return None;
            }
        }
    }
    None
}

/// Rough heuristic for common stdlib modules
fn is_likely_stdlib(top_level: &str) -> bool {
    const STDLIB: &[&str] = &[
        "abc",
        "argparse",
        "ast",
        "asyncio",
        "base64",
        "collections",
        "contextlib",
        "copy",
        "csv",
        "ctypes",
        "dataclasses",
        "datetime",
        "decimal",
        "difflib",
        "email",
        "enum",
        "functools",
        "glob",
        "hashlib",
        "hmac",
        "html",
        "http",
        "importlib",
        "inspect",
        "io",
        "itertools",
        "json",
        "logging",
        "math",
        "multiprocessing",
        "operator",
        "os",
        "pathlib",
        "pickle",
        "platform",
        "pprint",
        "queue",
        "random",
        "re",
        "secrets",
        "shutil",
        "signal",
        "socket",
        "sqlite3",
        "ssl",
        "string",
        "struct",
        "subprocess",
        "sys",
        "tempfile",
        "textwrap",
        "threading",
        "time",
        "timeit",
        "traceback",
        "typing",
        "unittest",
        "urllib",
        "uuid",
        "warnings",
        "weakref",
        "xml",
        "zipfile",
        "zlib",
        "builtins",
        "codecs",
        "concurrent",
        "configparser",
        "dis",
        "fractions",
        "ftplib",
        "getpass",
        "grp",
        "gzip",
        "heapq",
        "ipaddress",
        "lzma",
        "mimetypes",
        "numbers",
        "posixpath",
        "pwd",
        "selectors",
        "shelve",
        "smtplib",
        "statistics",
        "sysconfig",
        "tarfile",
        "tkinter",
        "trace",
        "types",
        "unicodedata",
    ];
    STDLIB.contains(&top_level)
}

// ---------------------------------------------------------------------------
// Linter integration
// ---------------------------------------------------------------------------

async fn run_linter(file_path: &Path, root: &Path) -> Result<String> {
    // Try ruff first (fast, Rust-based)
    let ruff_result = tokio::process::Command::new("ruff")
        .args(["check", "--output-format=json"])
        .arg(file_path)
        .current_dir(root)
        .output()
        .await;

    if let Ok(output) = ruff_result {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // ruff returns exit code 1 when issues found — that's normal
        if !stdout.is_empty() {
            if let Ok(issues) = serde_json::from_str::<Vec<Value>>(&stdout) {
                return format_ruff_output(&issues);
            }
        }
        if output.status.success() && stdout.trim().is_empty() {
            return Ok("No issues found (ruff).".to_string());
        }
        if stdout.trim() == "[]" {
            return Ok("No issues found (ruff).".to_string());
        }
        // If we got here with non-empty stdout but couldn't parse JSON,
        // just return the raw output
        if !stdout.is_empty() {
            return Ok(format!("## ruff output\n\n```\n{}\n```", stdout.trim()));
        }
        if !stderr.is_empty() {
            return Ok(format!("## ruff error\n\n```\n{}\n```", stderr.trim()));
        }
    }

    // Try flake8
    let flake8_result = tokio::process::Command::new("flake8")
        .args(["--format=json"])
        .arg(file_path)
        .current_dir(root)
        .output()
        .await;

    if let Ok(output) = flake8_result {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.is_empty() {
            return Ok(format!("## flake8 output\n\n```\n{}\n```", stdout.trim()));
        }
        if output.status.success() {
            return Ok("No issues found (flake8).".to_string());
        }
    }

    // Try pylint
    let pylint_result = tokio::process::Command::new("pylint")
        .args(["--output-format=json"])
        .arg(file_path)
        .current_dir(root)
        .output()
        .await;

    if let Ok(output) = pylint_result {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.is_empty() {
            return Ok(format!(
                "## pylint output\n\n```json\n{}\n```",
                stdout.trim()
            ));
        }
        if output.status.success() {
            return Ok("No issues found (pylint).".to_string());
        }
    }

    Ok("No linter found. Install `ruff` (recommended), `flake8`, or `pylint`.".to_string())
}

fn format_ruff_output(issues: &[Value]) -> Result<String> {
    if issues.is_empty() {
        return Ok("No issues found (ruff).".to_string());
    }

    let mut out = format!("## ruff: {} issue(s)\n\n", issues.len());
    out.push_str("| Line | Code | Message |\n");
    out.push_str("|------|------|---------|\n");

    for issue in issues {
        let line = issue
            .get("location")
            .and_then(|l| l.get("row"))
            .and_then(|r| r.as_u64())
            .unwrap_or(0);
        let code = issue.get("code").and_then(|c| c.as_str()).unwrap_or("?");
        let message = issue.get("message").and_then(|m| m.as_str()).unwrap_or("");
        out.push_str(&format!("| {} | {} | {} |\n", line, code, message));
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// LanguageService implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LanguageService for PythonService {
    fn name(&self) -> &str {
        "python"
    }

    fn is_applicable(&self, root: &Path) -> bool {
        root.join("pyproject.toml").exists()
            || root.join("requirements.txt").exists()
            || root.join("Pipfile").exists()
            || root.join("setup.py").exists()
            || root.join("setup.cfg").exists()
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        vec![
            // --- Group 1: Environment & Dependencies ---
            ToolDefinition {
                name: "python_read_manifest".to_string(),
                description: "List Python project dependencies from pyproject.toml (Poetry/PDM/PEP 621), Pipfile, or requirements.txt. Returns package names, versions, and dependency groups.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "python_resolve_import".to_string(),
                description: "Resolve a Python import path to its source file. Handles relative imports (dot notation), project-local modules, standard library, and third-party packages. Reports whether the import is local, stdlib, or site-packages.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "import_path": {
                            "type": "string",
                            "description": "Import path to resolve (e.g. 'os.path', '.models', '...utils.helpers')"
                        },
                        "from_file": {
                            "type": "string",
                            "description": "File path the import appears in (needed for relative imports)"
                        }
                    },
                    "required": ["import_path"]
                }),
            },
            // --- Group 2: Code Introspection ---
            ToolDefinition {
                name: "python_inspect_code".to_string(),
                description: "Parse a Python file with tree-sitter and return structural overview: all top-level classes (with bases, methods, fields, decorators) and functions (with parameters, return types, decorators).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Path to the Python file to inspect"
                        }
                    },
                    "required": ["file"]
                }),
            },
            // --- Group 3: Linting ---
            ToolDefinition {
                name: "python_run_linter".to_string(),
                description: "Run a Python linter (ruff preferred, fallback to flake8/pylint) on a file. Returns structured diagnostics: line, code, message.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Path to the Python file to lint"
                        }
                    },
                    "required": ["file"]
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
            "python_read_manifest" => self.tool_read_manifest(root).await,
            "python_resolve_import" => {
                let import_path = args
                    .get("import_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let from_file = args.get("from_file").and_then(|v| v.as_str()).unwrap_or("");
                self.tool_resolve_import(import_path, from_file, root).await
            }
            "python_inspect_code" => {
                let file_path = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
                self.tool_inspect_code(file_path, root).await
            }
            "python_run_linter" => {
                let file_path = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
                self.tool_run_linter(file_path, root).await
            }
            _ => anyhow::bail!("Unknown Python tool: {}", name),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

impl PythonService {
    async fn tool_read_manifest(&self, root: &Path) -> Result<String> {
        let mut out = String::new();

        // Environment info header
        out.push_str("## Python Environment\n\n");
        out.push_str(&format!("- **Package manager:** {}\n", self.env.manager));
        if let Some(ref manifest) = self.env.manifest_path {
            out.push_str(&format!(
                "- **Manifest:** `{}`\n",
                manifest.strip_prefix(root).unwrap_or(manifest).display()
            ));
        }
        if self.env.has_venv {
            let venv_display = self
                .env
                .venv_path
                .as_ref()
                .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
                .unwrap_or_else(|| "detected".to_string());
            out.push_str(&format!("- **Virtual env:** `{}`\n", venv_display));
        } else {
            out.push_str("- **Virtual env:** not detected\n");
        }
        out.push('\n');

        // Read and display dependencies
        let (summary, deps) = read_manifest(root, &self.env)?;
        out.push_str(&format!("## {}\n\n", summary));

        if deps.is_empty() {
            out.push_str("No dependencies found.\n");
            return Ok(out);
        }

        // Group by group
        let mut groups: std::collections::BTreeMap<String, Vec<&Dependency>> =
            std::collections::BTreeMap::new();
        for dep in &deps {
            groups.entry(dep.group.clone()).or_default().push(dep);
        }

        for (group, group_deps) in &groups {
            out.push_str(&format!("### {} ({})\n\n", group, group_deps.len()));
            out.push_str("| Package | Version |\n");
            out.push_str("|---------|--------|\n");
            for dep in group_deps {
                out.push_str(&format!("| {} | {} |\n", dep.name, dep.version));
            }
            out.push('\n');
        }

        Ok(out)
    }

    async fn tool_resolve_import(
        &self,
        import_path: &str,
        from_file: &str,
        root: &Path,
    ) -> Result<String> {
        if import_path.is_empty() {
            anyhow::bail!("import_path is required");
        }

        let from = if from_file.is_empty() {
            root.to_path_buf()
        } else {
            let p = PathBuf::from(from_file);
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
        };

        let mut out = String::new();
        out.push_str(&format!("## Resolving `{}`\n\n", import_path));

        match resolve_python_import(import_path, &from, root) {
            Some((path, kind)) => {
                let display_path = if path.starts_with("<") {
                    path.display().to_string()
                } else {
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string()
                };
                out.push_str(&format!("- **Resolved to:** `{}`\n", display_path));
                out.push_str(&format!("- **Kind:** {}\n", kind));

                // If it's a local file, show its exports (top-level names)
                if path.exists() && path.extension().map(|e| e == "py").unwrap_or(false) {
                    if let Ok(code) = tokio::fs::read_to_string(&path).await {
                        if let Ok((classes, functions)) = inspect_python_code(&code) {
                            if !classes.is_empty() || !functions.is_empty() {
                                out.push_str("\n### Available names\n\n");
                                for cls in &classes {
                                    out.push_str(&format!(
                                        "- `class {}` (line {})\n",
                                        cls.name, cls.line
                                    ));
                                }
                                for func in &functions {
                                    out.push_str(&format!(
                                        "- `def {}` (line {})\n",
                                        func.name, func.line
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            None => {
                out.push_str("**Could not resolve import.**\n\n");
                out.push_str("Possible reasons:\n");
                out.push_str("- Missing `__init__.py` in package directory\n");
                out.push_str("- Incorrect relative import depth\n");
                out.push_str("- Module not installed or not in PYTHONPATH\n");
            }
        }

        Ok(out)
    }

    async fn tool_inspect_code(&self, file_path: &str, root: &Path) -> Result<String> {
        if file_path.is_empty() {
            anyhow::bail!("file is required");
        }

        let path = PathBuf::from(file_path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };

        if !path.exists() {
            anyhow::bail!("File not found: {}", path.display());
        }

        let code = tokio::fs::read_to_string(&path).await?;
        let (classes, functions) = inspect_python_code(&code)?;

        let rel = path.strip_prefix(root).unwrap_or(&path);
        let mut out = format!("## `{}`\n\n", rel.display());

        if classes.is_empty() && functions.is_empty() {
            out.push_str("No top-level classes or functions found.\n");
            return Ok(out);
        }

        // Classes
        for cls in &classes {
            out.push_str(&format!("### class `{}`", cls.name));
            if !cls.bases.is_empty() {
                out.push_str(&format!("({})", cls.bases.join(", ")));
            }
            out.push_str(&format!(" — line {}\n\n", cls.line));

            if !cls.decorators.is_empty() {
                out.push_str("**Decorators:** ");
                let decs: Vec<String> =
                    cls.decorators.iter().map(|d| format!("`@{}`", d)).collect();
                out.push_str(&decs.join(", "));
                out.push_str("\n\n");
            }

            if !cls.fields.is_empty() {
                out.push_str("**Fields:**\n\n");
                out.push_str("| Name | Type |\n");
                out.push_str("|------|------|\n");
                for field in &cls.fields {
                    let ty = field.type_annotation.as_deref().unwrap_or("—");
                    out.push_str(&format!("| `{}` | `{}` |\n", field.name, ty));
                }
                out.push('\n');
            }

            if !cls.methods.is_empty() {
                out.push_str("**Methods:**\n\n");
                out.push_str("| Method | Parameters | Return | Decorators |\n");
                out.push_str("|--------|-----------|--------|------------|\n");
                for method in &cls.methods {
                    let ret = method.return_type.as_deref().unwrap_or("—");
                    let decs = if method.decorators.is_empty() {
                        "—".to_string()
                    } else {
                        method
                            .decorators
                            .iter()
                            .map(|d| format!("@{}", d))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    out.push_str(&format!(
                        "| `{}` | `{}` | `{}` | {} |\n",
                        method.name, method.params, ret, decs
                    ));
                }
                out.push('\n');
            }
        }

        // Top-level functions
        if !functions.is_empty() {
            out.push_str("### Top-level functions\n\n");
            out.push_str("| Function | Parameters | Return | Decorators |\n");
            out.push_str("|----------|-----------|--------|------------|\n");
            for func in &functions {
                let ret = func.return_type.as_deref().unwrap_or("—");
                let decs = if func.decorators.is_empty() {
                    "—".to_string()
                } else {
                    func.decorators
                        .iter()
                        .map(|d| format!("@{}", d))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                out.push_str(&format!(
                    "| `{}` | `{}` | `{}` | {} |\n",
                    func.name, func.params, ret, decs
                ));
            }
            out.push('\n');
        }

        Ok(out)
    }

    async fn tool_run_linter(&self, file_path: &str, root: &Path) -> Result<String> {
        if file_path.is_empty() {
            anyhow::bail!("file is required");
        }

        let path = PathBuf::from(file_path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };

        if !path.exists() {
            anyhow::bail!("File not found: {}", path.display());
        }

        run_linter(&path, root).await
    }
}
