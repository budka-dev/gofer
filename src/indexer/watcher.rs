use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::parser::SupportedLanguage;

/// Task for the indexer worker
#[derive(Debug, Clone)]
pub enum IndexTask {
    Reindex(PathBuf),
    Delete(PathBuf),
}

/// gofer configuration from config.toml
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct GoferConfig {
    #[serde(default)]
    pub indexer: IndexerConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct IndexerConfig {
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub parallel_workers: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EmbeddingConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub external_api_key: Option<String>,
    #[serde(default)]
    pub external_model: Option<String>,
    #[serde(default)]
    pub dimensions: Option<usize>,
}

fn default_provider() -> String { "external".to_string() }
fn default_batch_size() -> usize {
    32
}
fn default_pool_size() -> usize {
    4
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            batch_size: default_batch_size(),
            pool_size: default_pool_size(),
            external_url: None,
            external_api_key: None,
            external_model: None,
            dimensions: None,
        }
    }
}

/// Load gofer configuration from .gofer/config.toml
pub fn load_config(gofer_dir: &Path) -> GoferConfig {
    let config_path = gofer_dir.join("config.toml");
    if !config_path.exists() {
        return GoferConfig::default();
    }

    match std::fs::read_to_string(&config_path) {
        Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse config.toml: {}", e);
            GoferConfig::default()
        }),
        Err(e) => {
            tracing::warn!("Failed to read config.toml: {}", e);
            GoferConfig::default()
        }
    }
}

/// Build gitignore matcher for a directory
fn build_gitignore(root: &Path, extra_ignores: &[String]) -> Gitignore {
    let gitignore_path = root.join(".gitignore");
    let mut builder = GitignoreBuilder::new(root);

    // Add default ignores (always ignored)
    let _ = builder.add_line(None, ".git");
    let _ = builder.add_line(None, ".gofer");
    let _ = builder.add_line(None, ".qoder");
    let _ = builder.add_line(None, "*.lock");

    // Add user-configured ignores from config.toml
    for pattern in extra_ignores {
        let _ = builder.add_line(None, pattern);
    }

    // Add project .gitignore if exists
    if gitignore_path.exists() {
        let _ = builder.add(&gitignore_path);
    }

    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Check if a file should be indexed based on extension
fn should_index(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(SupportedLanguage::from_extension)
        .is_some()
}

pub fn find_watchable_dirs(root: &Path, extra_ignores: &[String]) -> Vec<PathBuf> {
    use ignore::WalkBuilder;

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|e| {
            // Only yield directories for watch targets
            e.file_type().is_some_and(|ft| ft.is_dir())
        });

    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for pattern in extra_ignores {
        let ignore_pattern = if pattern.starts_with('!') {
            pattern.clone()
        } else {
            format!("!{}", pattern)
        };
        let _ = overrides.add(&ignore_pattern);
    }
    if let Ok(ov) = overrides.build() {
        builder.overrides(ov);
    }

    let mut dirs = Vec::new();
    for entry in builder.build().flatten() {
        let path = entry.path().to_path_buf();
        if path.is_dir() {
            dirs.push(path);
        }
    }

    dirs
}

/// Start the file watcher
pub async fn start_watcher(
    root_path: PathBuf,
    task_tx: mpsc::Sender<Vec<IndexTask>>,
    extra_ignores: Vec<String>,
    cancel: CancellationToken,
) {
    let gitignore = build_gitignore(&root_path, &extra_ignores);

    // Spawn blocking prevents the rx.recv_timeout loop from starving the tokio reactor.
    tokio::task::spawn_blocking(move || {
        let (tx, rx) = channel::<DebounceEventResult>();

        // Create debouncer with 500ms delay
        let mut debouncer = match new_debouncer(Duration::from_millis(500), tx) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to create file watcher: {}", e);
                return;
            }
        };

        // Watch explicit non-ignored directories instead of using RecursiveMode::Recursive on the root.
        // This dramatically reduces OS inotify watch handle consumption by not watching node_modules.
        let dirs = find_watchable_dirs(&root_path, &extra_ignores);
        tracing::info!(
            "Watcher registering {} non-ignored directories explicitly",
            dirs.len()
        );
        for dir in dirs {
            if let Err(e) = debouncer.watcher().watch(&dir, RecursiveMode::NonRecursive) {
                tracing::debug!("Watcher failed to register dir {:?}: {}", dir, e);
            }
        }

        tracing::info!("File watcher started for: {:?}", root_path);

        // Process events in a loop
        loop {
            if cancel.is_cancelled() {
                tracing::info!("Watcher stopped for: {:?}", root_path);
                break;
            }

            // Use recv_timeout so we can check cancellation periodically
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(events)) => {
                    let mut batch = Vec::new();
                    for event in events {
                        let path = event.path;

                        // Skip ignored files
                        if gitignore.matched(&path, path.is_dir()).is_ignore() {
                            continue;
                        }

                        // Dynamically add new valid directories to watcher
                        if path.is_dir() {
                            let _ = debouncer
                                .watcher()
                                .watch(&path, RecursiveMode::NonRecursive);
                            continue;
                        }

                        // Skip non-indexable files
                        if !should_index(&path) {
                            continue;
                        }

                        // Determine task type based on file existence
                        let task = if path.exists() {
                            IndexTask::Reindex(path.clone())
                        } else {
                            IndexTask::Delete(path.clone())
                        };

                        tracing::debug!("File event: {:?}", task);
                        batch.push(task);
                    }

                    if !batch.is_empty() {
                        // Send the entire batch at once
                        if task_tx.blocking_send(batch).is_err() {
                            tracing::warn!("Indexer channel closed");
                            return;
                        }
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!("Watcher error: {:?}", e);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("Watcher channel closed");
                    break;
                }
            }
        }
    }); // No await, spawn_blocking detaches. We just let it run until cancelled.
}

/// Scan directory and return all indexable files.
/// Uses `ignore::WalkBuilder` for parallel traversal with built-in .gitignore support.
pub fn scan_directory(root: &Path, extra_ignores: &[String]) -> Vec<PathBuf> {
    use ignore::WalkBuilder;

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true) // Пропускать скрытые файлы
        .git_ignore(true) // Учитывать .gitignore
        .git_global(true) // Учитывать глобальный gitignore
        .git_exclude(true) // Учитывать .git/info/exclude
        .threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        );

    // Добавить пользовательские ignore-паттерны через overrides (см. выше)
    // builder.add_custom_ignore_filename(pattern); // Удалено - неправильное использование

    // Также добавить через Override для glob-паттернов
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for pattern in extra_ignores {
        // OverrideBuilder treats normal patterns as allow-lists.
        // To ignore, we need to prefix the pattern with '!'
        let ignore_pattern = if pattern.starts_with('!') {
            pattern.clone()
        } else {
            format!("!{}", pattern)
        };
        let _ = overrides.add(&ignore_pattern);
    }
    if let Ok(ov) = overrides.build() {
        builder.overrides(ov);
    }

    let mut files = Vec::new();

    for entry in builder.build().flatten() {
        let path = entry.path().to_path_buf();
        if path.is_file() && should_index(&path) {
            files.push(path);
        }
    }

    files
}

// scan_recursive больше не используется — заменён на ignore::WalkBuilder
