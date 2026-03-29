use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::CacheManager;
use crate::error_recovery::CircuitBreaker;
use crate::indexer::EmbedderPool;
use crate::languages::{generic_lsp::GenericLspClient, LanguageService};
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
    pub lang_manager: Arc<crate::indexer::parser::lang_manager::LanguageManager>,
    pub lsp_clients: Arc<RwLock<std::collections::HashMap<String, Arc<GenericLspClient>>>>,
    /// Language-specific services (Vue, TypeScript, Python, etc.)
    pub language_services: Arc<Vec<Box<dyn LanguageService>>>,
    pub state: Arc<crate::daemon::state::DaemonState>,
}

impl ToolContext {
    /// Get or initialize an LSP client for this file based on its extension
    #[allow(dead_code)]
    pub async fn get_lsp_client(&self, file_path: &str) -> anyhow::Result<Option<Arc<GenericLspClient>>> {
        let ext = std::path::Path::new(file_path).extension().and_then(|e| e.to_str()).unwrap_or("");
        
        // 1. Resolve Language via lang_manager
        let lang_name = match self.lang_manager.get_language_by_ext(ext) {
            Some(l) => l,
            None => return Ok(None) // No language support
        };
        
        // 2. Load manifest to find LSP details
        let loaded_lang = match self.lang_manager.loaded_langs.get(&lang_name) {
            Some(l) => l,
            None => return Ok(None)
        };
        
        let lsp_config = match &loaded_lang.value().manifest.lsp {
            Some(config) => config.clone(),
            None => return Ok(None) // No LSP configured for this language
        };

        let lang_id = lsp_config.name.clone().unwrap_or_else(|| lang_name.clone());

        // Fast path: already initialized
        {
            let clients_guard = self.lsp_clients.read().await;
            if let Some(client) = clients_guard.get(&lang_id) {
                if client.is_ready().await {
                    return Ok(Some(client.clone()));
                }
            }
        }

        // Slow path: initialize LSP client
        let mut clients_guard = self.lsp_clients.write().await;

        // Double-check in case another task initialized it
        if let Some(client) = clients_guard.get(&lang_id) {
            if client.is_ready().await {
                return Ok(Some(client.clone()));
            }
        }

        let command_str;
        let mut tool_args = Vec::new();

        if let Some(c) = &lsp_config.command {
            command_str = c.clone();
        } else if let Some(t) = &lsp_config.tool {
            if let Some(tool_manifest) = self.lang_manager.get_tool(t) {
                if let Some(lsp_tool_config) = &tool_manifest.lsp {
                    let mut cmd = lsp_tool_config.command.clone();
                    
                    if let Some(install) = &tool_manifest.tool.install {
                        let exe_name = &install.binary.executable_name;
                        let local_exe = self.lang_manager.tools_dir.join(t).join("bin").join(exe_name);
                        
                        if !local_exe.exists() {
                            tracing::info!("Tool executable {} not found locally. Attempting to download...", exe_name);
                            if let Ok(path) = self.lang_manager.download_and_extract_binary(t, &tool_manifest).await {
                                cmd = path.to_string_lossy().to_string();
                            }
                        } else {
                            cmd = local_exe.to_string_lossy().to_string();
                        }
                    }

                    command_str = cmd;
                    tool_args = lsp_tool_config.args.clone();
                } else {
                    tracing::warn!("Tool {} does not have LSP capabilities", t);
                    return Ok(None);
                }
            } else {
                tracing::warn!("Tool {} not found in lang-hub", t);
                return Ok(None);
            }
        } else {
            return Ok(None);
        }

        // Start new instance
        let shell_args = shell_words::split(&command_str)
            .unwrap_or_else(|_| vec![command_str.clone()]);
        let cmd = shell_args[0].clone();
        let mut args: Vec<String> = shell_args.into_iter().skip(1).collect();
        args.extend(tool_args);

        let mut actual_root = self.root_path.as_ref().clone();
        if let Ok(file_path_buf) = resolve_path_buf(&self.root_path, file_path) {
            actual_root = find_project_root(
                &file_path_buf,
                &loaded_lang.value().manifest.language.root_markers,
                &self.root_path
            );
        }

        let init_options = lsp_config.init_options.clone();

        let client = Arc::new(GenericLspClient::new(
            actual_root,
            cmd,
            args,
            lang_id.clone(),
            init_options,
        ));
        
        client.start().await?;
        clients_guard.insert(lang_id, client.clone());

        Ok(Some(client))
    }
}

/// Finds the specific project root for a file by traversing upwards looking for root markers.
pub fn find_project_root(start_path: &Path, markers: &[String], fallback_root: &Path) -> PathBuf {
    if markers.is_empty() {
        return fallback_root.to_path_buf();
    }

    let mut current_dir = if start_path.is_file() {
        start_path.parent()
    } else {
        Some(start_path)
    };

    while let Some(dir) = current_dir {
        for marker in markers {
            if dir.join(marker).exists() {
                return dir.to_path_buf();
            }
        }
        current_dir = dir.parent();
    }

    fallback_root.to_path_buf()
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
