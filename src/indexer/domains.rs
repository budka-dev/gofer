//! Domain / tech-stack classification from path + content.
//! Not used on the search MCP hot path (columns kept as "unknown"); retained for optional future use.

#![allow(dead_code)]

use smol_str::SmolStr;

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
            py_paths: vec!["python/".into(), "app/".into(), "src/".into()],
            frontend_paths: vec![
                "frontend/".into(),
                "ui/".into(),
                "client/".into(),
                "src/".into(),
            ],
            ops_paths: vec!["ops/".into(), "deploy/".into(), "scripts/".into()],
            shared_paths: vec!["shared/".into(), "common/".into(), "models/".into()],
        }
    }
}

/// Detect domain and tech stack from file path and content.
pub fn detect_domain(
    path: &str,
    content: &str,
    config: &DomainConfig,
) -> (SmolStr, Vec<SmolStr>) {
    let mut domain = Domain::Unknown;
    let mut tech_stack = Vec::new();

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

    // Content-based fallback for unknown
    if domain == Domain::Unknown {
        if content.contains("fn main") || content.contains("pub fn") || content.contains("use std::")
        {
            domain = Domain::Rust;
            tech_stack.push("rust".into());
        } else if content.contains("def ") || content.contains("import ") {
            domain = Domain::Python;
            tech_stack.push("python".into());
        }
    }

    (domain.as_str().into(), tech_stack)
}
