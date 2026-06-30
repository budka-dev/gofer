use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::CacheManager;
use crate::error_recovery::CircuitBreaker;
use crate::indexer::EmbedderPool;
use crate::storage::{LanceStorage, SqliteStorage};

/// Context for executing tools — Arc-wrapped resources for cloning across async tasks.
#[derive(Clone)]
pub struct ToolContext {
    pub sqlite: Arc<SqliteStorage>,
    pub lance: Arc<LanceStorage>,
    pub embedder: Arc<EmbedderPool>,
    pub root_path: Arc<PathBuf>,
    pub cache: Arc<CacheManager>,
    pub embedding_circuit: Arc<CircuitBreaker>,
    pub vector_circuit: Arc<CircuitBreaker>,
}

/// Резолвинг пути: если путь относительный, превращает в абсолютный через root_path.
pub fn resolve_path(root: &Path, file: &str) -> String {
    let p = Path::new(file);
    if p.is_absolute() {
        file.to_string()
    } else {
        root.join(file).to_string_lossy().to_string()
    }
}

use std::path::Component;

/// Normalize a path (lexically resolve ".." and ".")
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}

/// Securely resolve path, preventing Path Traversal
pub fn resolve_path_buf(root: &Path, file: &str) -> anyhow::Result<PathBuf> {
    let p = Path::new(file);
    let absolute = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };

    let normalized = normalize_path(&absolute);
    let normalized_root = normalize_path(root);

    if normalized.starts_with(&normalized_root) {
        Ok(normalized)
    } else {
        Err(anyhow::anyhow!("Path traversal attempt blocked: {}", file))
    }
}

/// Strip root_path prefix from an absolute file path, returning a relative path.
pub fn make_relative(root: &Path, abs_path: &str) -> String {
    Path::new(abs_path)
        .strip_prefix(root)
        .ok()
        .and_then(|p| p.to_str())
        .unwrap_or(abs_path)
        .to_string()
}

/// make_relative для PathBuf
pub fn make_relative_pathbuf(root: &Path, abs_path: &Path) -> String {
    abs_path
        .strip_prefix(root)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| abs_path.to_string_lossy().to_string())
}
