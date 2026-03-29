#![allow(dead_code, unused_imports, unused_variables)]
use crate::models::chunk::TypeField;
use regex::Regex;
use smol_str::SmolStr;
use std::collections::HashSet;

/// Domain classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Rust,
    Python,
    Frontend,
    Shared,
    Ops,
    Unknown,
}

impl Domain {
    pub fn as_str(&self) -> &'static str {
        match self {
            Domain::Rust => "backend",
            Domain::Python => "backend",
            Domain::Frontend => "frontend",
            Domain::Shared => "shared",
            Domain::Ops => "ops",
            Domain::Unknown => "unknown",
        }
    }
}

/// Domain detection configuration
#[derive(Debug, Clone, Default)]
pub struct DomainConfig {
    pub rs_paths: Vec<String>,
    pub py_paths: Vec<String>,
    pub frontend_paths: Vec<String>,
    pub ops_paths: Vec<String>,
    pub shared_paths: Vec<String>,
}

impl DomainConfig {
    pub fn default_config() -> Self {
        Self {
            rs_paths: vec![
                "backend/".into(),
                "server/".into(),
                "api/".into(),
                "src-rust/".into(),
                "src/".into(),
            ],
            py_paths: vec![
                "python/".into(),
                "app/".into(),
                "src/".into(),
            ],
            frontend_paths: vec![
                "frontend/".into(),
                "ui/".into(),
                "client/".into(),
                "src/".into(),
            ],
            ops_paths: vec![
                "ops/".into(),
                "deploy/".into(),
                "scripts/".into(),
            ],
            shared_paths: vec![
                "shared/".into(),
                "common/".into(),
                "models/".into(),
            ],
        }
    }
}

/// Detect domain and tech stack from file path and content
pub fn detect_domain(
    path: &str,
    content: &str,
    config: &DomainConfig,
) -> (SmolStr, Vec<SmolStr>) {
    let mut domain = Domain::Unknown;
    let mut tech_stack = Vec::new();

    // 1. Path-based detection
    if config.rs_paths.iter().any(|p| path.contains(p)) {
        domain = Domain::Rust;
        tech_stack.push("rust".into());
    } else if config.py_paths.iter().any(|p| path.contains(p)) {
        domain = Domain::Python;
        tech_stack.push("python".into());
    } else if config.frontend_paths.iter().any(|p| path.contains(p)) {
        domain = Domain::Frontend;
        if path.ends_with(".vue") {
            tech_stack.push("vue".into());
        } else if path.ends_with(".ts") || path.ends_with(".tsx") {
            tech_stack.push("typescript".into());
        }
    } else if config.ops_paths.iter().any(|p| path.contains(p)) {
        domain = Domain::Ops;
    } else if config.shared_paths.iter().any(|p| path.contains(p)) {
        domain = Domain::Shared;
    }

    // 2. Content-based detection (fallback or enrichment)
    if domain == Domain::Unknown {
        if content.contains("fn main") || content.contains("pub struct") {
            domain = Domain::Rust;
            tech_stack.push("rust".into());
        } else if (content.contains("def ") || content.contains("import "))
            && path.ends_with(".py") {
                domain = Domain::Python;
                tech_stack.push("python".into());
            }
    }

    (domain.as_str().into(), tech_stack)
}

/// Find routes in backend files
pub fn parse_backend_routes(content: &str, language: &str) -> Vec<(String, String)> {
    let mut routes = Vec::new();
    if language == "rust" {
        // Simple regex for common web frameworks
        let re = Regex::new(r#"(?i)@\[(get|post|put|delete)\("([^"]+)"\)\]"#).unwrap();
        for cap in re.captures_iter(content) {
            routes.push((cap[1].to_uppercase(), cap[2].to_string()));
        }
    }
    routes
}

/// Find API calls in frontend files
pub fn parse_frontend_api_calls(content: &str, language: &str) -> Vec<(String, String)> {
    let mut calls = Vec::new();
    if language == "typescript" || language == "javascript" || language == "vue" {
        let re = Regex::new(r#"(?i)api\.(get|post|put|delete)\("([^"]+)"\)"#).unwrap();
        for cap in re.captures_iter(content) {
            calls.push((cap[1].to_uppercase(), cap[2].to_string()));
        }
    }
    calls
}

/// Check if two paths match (allowing for simple patterns)
pub fn paths_match(pattern: &str, path: &str) -> bool {
    let p = pattern.replace(':', "([^/]+)");
    let re = Regex::new(&format!("^{}$", p)).unwrap();
    re.is_match(path)
}

// ---------------------------------------------------------------------------
// Structural Links (Cross-stack)
// ---------------------------------------------------------------------------

/// Результат Jaccard-сравнения двух наборов полей
#[derive(Debug, Clone)]
struct JaccardResult {
    similarity: f64,
    matched_fields: Vec<String>,
}

/// Вычисляет Jaccard similarity между двумя наборами полей.
fn jaccard_type_fields(fields1: &[TypeField], fields2: &[TypeField]) -> JaccardResult {
    let set1: HashSet<String> = fields1.iter().map(|f| f.normalized.clone()).collect();
    let set2: HashSet<String> = fields2.iter().map(|f| f.normalized.clone()).collect();

    let intersection: HashSet<_> = set1.intersection(&set2).cloned().collect();
    let union: HashSet<_> = set1.union(&set2).cloned().collect();

    if union.is_empty() {
        return JaccardResult {
            similarity: 0.0,
            matched_fields: Vec::new(),
        };
    }

    let matched_fields = fields1
        .iter()
        .filter(|f| intersection.contains(&f.normalized))
        .map(|f| f.name.clone())
        .collect();

    JaccardResult {
        similarity: intersection.len() as f64 / union.len() as f64,
        matched_fields,
    }
}

// === AST-based Structural Fingerprinting ===

use super::parser::{parse_all_type_fields, SupportedLanguage};
use crate::storage::SqliteStorage;

/// Результат Jaccard-сравнения двух наборов полей
#[derive(Debug, Clone)]
pub struct FieldMatch {
    pub similarity: f64,
    pub matched_fields: Vec<String>,
}

/// Выполняет структурный фингерпринтинг: сбор полей типов и поиск кросс-стековых связей.
pub async fn run_structural_fingerprinting(
    parsed_files: &[(String, String, &SupportedLanguage)],
    sqlite: &SqliteStorage,
) -> anyhow::Result<usize> {
    // Фаза 1: Извлекаем fingerprints
    let mut rust_types: Vec<(String, String, Vec<TypeField>)> = Vec::new(); // (file_path, type_name, fields)
    let mut ts_types: Vec<(String, String, Vec<TypeField>)> = Vec::new();

    for (path, content, language) in parsed_files {
        let all_types = match parse_all_type_fields(content, (*language).clone()) {
            Ok(t) => t,
            Err(_) => continue,
        };

        for (type_name, fields) in all_types {
            if fields.len() < 3 {
                continue; // Пропускаем тривиальные типы
            }

            // Сохраняем fingerprint в SQLite
            if let Ok(Some(file)) = sqlite.get_file(path).await {
                if let Ok(symbols) = sqlite.get_symbol_by_name(&type_name).await {
                    if let Some(symbol) = symbols.iter().find(|s| s.file_id == file.id) {
                        let fields_json = serde_json::to_string(
                            &fields.iter().map(|f| &f.name).collect::<Vec<_>>(),
                        )
                        .unwrap_or_default();
                        let fields_normalized = serde_json::to_string(
                            &fields.iter().map(|f| &f.normalized).collect::<Vec<_>>(),
                        )
                        .unwrap_or_default();

                        let _ = sqlite
                            .upsert_type_fingerprint(
                                file.id,
                                symbol.id,
                                &type_name,
                                match language.name() {
                                    "rust" => "rust",
                                    "typescript"
                                    | "javascript"
                                    | "vue" => "typescript",
                                    "python" => "python",
                                    "go" => "go",
                                    _ => "unknown",
                                },
                                &fields_json,
                                &fields_normalized,
                                fields.len() as i32,
                            )
                            .await;
                    }
                }
            }

            match language.name() {
                "rust" => {
                    rust_types.push((path.clone(), type_name, fields));
                }
                "typescript"
                | "javascript"
                | "vue" => {
                    ts_types.push((path.clone(), type_name, fields));
                }
                "python" => {
                    rust_types.push((path.clone(), type_name, fields));
                }
                "go" => {
                    rust_types.push((path.clone(), type_name, fields));
                }
                _ => {}
            }
        }
    }

    // Фаза 2: Jaccard-сравнение rust <-> ts
    sqlite.clear_structural_links().await?;

    let mut links_created = 0;
    let threshold = 0.75;

    for (rust_path, rust_name, rust_fields) in &rust_types {
        for (ts_path, ts_name, ts_fields) in &ts_types {
            let m = jaccard_type_fields(rust_fields, ts_fields);

            if m.similarity >= threshold && !m.matched_fields.is_empty() {
                let metadata = serde_json::json!({
                    "matched_fields": m.matched_fields,
                    "jaccard": m.similarity,
                    "rust_field_count": rust_fields.len(),
                    "ts_field_count": ts_fields.len(),
                });

                let _ = sqlite
                    .upsert_cross_stack_link(
                        rust_path,
                        ts_path,
                        rust_name,
                        ts_name,
                        "structural",
                        m.similarity,
                        &metadata.to_string(),
                    )
                    .await;
                links_created += 1;
            }
        }
    }

    Ok(links_created)
}
