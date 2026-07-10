use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::embedder::EmbedderPool;
use super::parser::{smart_chunk_file, CodeParser, SupportedLanguage};
use super::pipeline;
use super::watcher::IndexTask;
use crate::cache::CacheManager;
use crate::daemon::state::SyncProgress;
use crate::models::{Symbol, SymbolReference};
use crate::storage::{LanceStorage, SqliteStorage};

/// Indexer service that processes files and updates storage
#[derive(Clone)]
pub struct IndexerService {
    sqlite: SqliteStorage,
    lance: Arc<LanceStorage>,
    embedder: Arc<EmbedderPool>,
    cache: Option<Arc<CacheManager>>,
    parallel_workers: usize,
}

impl IndexerService {
    pub fn new(
        sqlite: SqliteStorage,
        lance: Arc<LanceStorage>,
        embedder: Arc<EmbedderPool>,
        parallel_workers: usize,
    ) -> Self {
        Self {
            sqlite,
            lance,
            embedder,
            cache: None,
            parallel_workers,
        }
    }

    /// Set cache manager for invalidation on file changes (Feature 012)
    pub fn with_cache(mut self, cache: Arc<CacheManager>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub async fn run(self, mut rx: mpsc::Receiver<Vec<IndexTask>>) {
        tracing::info!(
            "Indexer service started with {} workers (used for SEDA)",
            self.parallel_workers
        );

        while let Some(batch) = rx.recv().await {
            let mut to_reindex = Vec::new();
            let mut to_delete = Vec::new();
            for task in batch {
                match task {
                    IndexTask::Reindex(path) => to_reindex.push(path),
                    IndexTask::Delete(path) => to_delete.push(path),
                }
            }

            let mut needs_resolve = false;

            // Process deletes individually
            for path in to_delete {
                if let Err(e) = self.delete_file(&path).await {
                    tracing::error!("Failed to delete {:?}: {}", path, e);
                } else {
                    needs_resolve = true;
                }
            }

            // Process reindexing
            if to_reindex.len() >= 10 {
                // Large burst: Hand off to SEDA pipeline for max throughput and batching
                tracing::info!(
                    "Event storm detected: Batch indexing {} files",
                    to_reindex.len()
                );
                let service = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = pipeline::run_pipeline(
                        to_reindex,
                        service.sqlite,
                        service.lance,
                        service.embedder,
                        None,
                        CancellationToken::new(),
                    )
                    .await
                    {
                        tracing::error!("Batched pipeline index failed: {}", e);
                    }
                });
            } else {
                // Small trickle: Process sequentially to eliminate SQLite lock contention
                for path in to_reindex {
                    if let Err(e) = self.index_file(&path).await {
                        tracing::error!("Failed to index {:?}: {}", path, e);
                    } else {
                        needs_resolve = true;
                    }
                }
            }

            if needs_resolve {
                match self.sqlite.resolve_references().await {
                    Ok(resolved) => {
                        if resolved > 0 {
                            tracing::debug!("Sequentially resolved {} references", resolved);
                        }
                    }
                    Err(e) => tracing::error!("Failed to resolve references sequentially: {:?}", e),
                }
            }
        }

        tracing::info!("Indexer service stopped");
    }

    /// Index a single file (used by watch mode and reindex RPC)
    pub async fn index_file(&self, path: &Path) -> anyhow::Result<()> {
        let content = tokio::fs::read_to_string(path).await?;
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let path_str = path.to_string_lossy().to_string();

        if !self.sqlite.needs_reindex(&path_str, &hash).await? {
            tracing::debug!("Skipping {:?} (unchanged)", path);
            return Ok(());
        }

        tracing::info!("Indexing: {:?}", path);

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some(language) = SupportedLanguage::from_extension(ext) else {
            return Ok(());
        };

        let mut parser = CodeParser::new();
        let symbols = parser.parse_symbols(&content, language.clone())?;
        let chunks = smart_chunk_file(&content, &path_str, language.clone()).unwrap_or_else(|e| {
            tracing::debug!(
                "smart_chunk_file failed for {}: {}, falling back to parse_chunks",
                path_str,
                e
            );
            parser
                .parse_chunks(&content, &path_str, language.clone())
                .unwrap_or_else(|e2| {
                    tracing::warn!("parse_chunks also failed for {}: {}", path_str, e2);
                    Vec::new()
                })
        });
        let all_refs = parser.parse_references(&content, language.clone())?;

        let modified = tokio::fs::metadata(path)
            .await?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        // Perform LanceDB write FIRST. If this fails, we want to return early (via ?)
        // so that we don't pollute SQLite with orphan metadata. This prevents split-brain.
        if !chunks.is_empty() {
            let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
            let embeddings = self.embedder.embed(texts).await?;
            self.lance.upsert_chunks(&chunks, &embeddings).await?;
        }

        // SQLite operations safely execute AFTER LanceDB succeeds
        let file_id = self.sqlite.upsert_file(&path_str, modified, &hash).await?;

        let symbols_with_file_id: Vec<Symbol> = symbols
            .into_iter()
            .map(|mut s| {
                s.file_id = file_id;
                s
            })
            .collect();

        self.sqlite
            .insert_symbols(file_id, &symbols_with_file_id)
            .await?;

        let stored_symbols = self.sqlite.get_file_symbols(file_id).await?;

        for symbol in &stored_symbols {
            let symbol_refs: Vec<SymbolReference> = all_refs
                .iter()
                .filter(|r| {
                    r.line >= symbol.line_start
                        && r.line <= symbol.line_end
                        && r.target_name != symbol.name
                })
                .cloned()
                .collect();

            if !symbol_refs.is_empty() {
                self.sqlite
                    .insert_references(symbol.id, &symbol_refs)
                    .await?;
            }
        }
        tracing::info!(
            "⚡ Incrementally indexed {:?}: {} symbols, {} chunks, {} refs",
            path,
            symbols_with_file_id.len(),
            chunks.len(),
            all_refs.len()
        );

        // Feature 012: Invalidate cache for changed file
        if let Some(ref cache) = self.cache {
            cache.invalidate_file(&path_str).await;
            cache.invalidate_all_searches().await;
            tracing::debug!("Invalidated cache for {:?}", path);
        }

        Ok(())
    }

    /// Delete a file from indices
    async fn delete_file(&self, path: &Path) -> anyhow::Result<()> {
        let path_str = path.to_string_lossy().to_string();
        self.sqlite.delete_file(&path_str).await?;
        {
            self.lance.delete_file(&path_str).await?;
        }

        // Feature 012: Invalidate cache for deleted file
        if let Some(ref cache) = self.cache {
            cache.invalidate_file(&path_str).await;
            cache.invalidate_all_searches().await;
            tracing::debug!("Invalidated cache for deleted file {:?}", path);
        }

        tracing::info!("Deleted: {:?}", path);
        Ok(())
    }

    /// Perform initial full sync using SEDA pipeline
    pub async fn full_sync(
        &self,
        root: &Path,
        extra_ignores: &[String],
        progress: Option<Arc<SyncProgress>>,
        metrics: Option<Arc<crate::daemon::state::DaemonMetrics>>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let sync_start = std::time::Instant::now();
        tracing::info!("Starting full sync for: {:?}", root);

        if let Some(ref p) = progress {
            p.reset();
            *p.stage.lock().await = "scanning".into();
        }

        // Arc clones — no ownership transfer, no loss on error
        let files = crate::indexer::watcher::scan_directory(root, extra_ignores);
        let metadata: Vec<pipeline::ParsedFileMetadata> = pipeline::run_pipeline(
            files,
            self.sqlite.clone(),
            self.lance.clone(),
            self.embedder.clone(),
            progress.clone(),
            cancel.clone(),
        )
        .await?;

        let changed_count = metadata.len();
        if changed_count == 0 {
            tracing::info!("No changes detected");
            return Ok(());
        }

        // Build vector index for ANN search (incremental)
        {
            if let Err(e) = self
                .lance
                .create_vector_index_incremental(Some(&self.sqlite))
                .await
            {
                tracing::warn!("Failed to create vector index: {}", e);
            }
        }

        // Monorepo / sub-project detection
        if let Some(ref p) = progress {
            *p.stage.lock().await = "monorepo detection".into();
        }
        tracing::info!("Sub-project detection...");
        let subproject_count = detect_and_store_subprojects(root, &self.sqlite).await;
        tracing::info!(
            "Sub-project detection: {} sub-projects found",
            subproject_count
        );

        tracing::info!("Full sync completed: {} files indexed", changed_count);

        if let Some(ref m) = metrics {
            let chunks_embedded = progress
                .as_ref()
                .map(|p| p.chunks_embedded.load(std::sync::atomic::Ordering::Relaxed))
                .unwrap_or(0);
            m.record_sync(
                changed_count,
                chunks_embedded,
                sync_start.elapsed().as_millis() as usize,
            );
        }

        if let Some(ref p) = progress {
            p.finish();
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Monorepo / sub-project detection
// ---------------------------------------------------------------------------

/// Manifest files that mark a sub-project root.
const MANIFEST_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", "cargo"),
    ("package.json", "npm"),
    ("go.mod", "go"),
    ("pyproject.toml", "python"),
];

/// Scan the project tree for manifest files and store discovered sub-projects.
async fn detect_and_store_subprojects(root: &Path, sqlite: &SqliteStorage) -> usize {
    sqlite.clear_subprojects().await.ok();

    let mut count = 0usize;
    let mut subprojects: Vec<(String, String, String, Option<String>)> = Vec::new(); // (name, rel_path, kind, parent)

    // Phase 1: Resolve npm/pnpm/yarn workspaces from root manifests
    let workspace_dirs = resolve_npm_workspaces(root).await;

    for ws_dir in &workspace_dirs {
        let rel_path = match ws_dir.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        let name = ws_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        // Read package name from package.json if available
        let pkg_name = ws_dir.join("package.json");
        let display_name = if pkg_name.exists() {
            tokio::fs::read_to_string(&pkg_name)
                .await
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| v.get("name")?.as_str().map(String::from))
                .unwrap_or(name)
        } else {
            name
        };
        subprojects.push((display_name, rel_path, "npm".to_string(), None));
    }

    // Phase 2: Walk directory tree looking for additional manifest files
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .build();

    let workspace_rel: std::collections::HashSet<String> =
        subprojects.iter().map(|(_, p, _, _)| p.clone()).collect();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        for &(manifest, kind) in MANIFEST_FILES {
            if file_name != manifest {
                continue;
            }

            let dir = match path.parent() {
                Some(d) => d,
                None => continue,
            };

            // Skip the root itself
            if dir == root {
                continue;
            }

            let rel_path = match dir.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            // Skip if already detected via workspace resolution
            if workspace_rel.contains(&rel_path) {
                continue;
            }

            // Skip node_modules, .git, vendor, target, etc.
            if rel_path.contains("node_modules")
                || rel_path.contains(".git")
                || rel_path.contains("vendor")
                || rel_path.contains("target")
                || rel_path.contains("__pycache__")
            {
                continue;
            }

            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            subprojects.push((name, rel_path, kind.to_string(), None));
        }
    }

    // Detect parent relationships
    let paths: Vec<String> = subprojects.iter().map(|(_, p, _, _)| p.clone()).collect();
    for sp in &mut subprojects {
        for candidate in &paths {
            if candidate != &sp.1 && sp.1.starts_with(candidate) {
                sp.3 = Some(candidate.clone());
                break;
            }
        }
    }

    // Store in SQLite
    for (name, rel_path, kind, parent) in &subprojects {
        match sqlite
            .upsert_subproject(name, rel_path, kind, parent.as_deref())
            .await
        {
            Ok(_id) => count += 1,
            Err(e) => tracing::warn!("Failed to store subproject {}: {}", rel_path, e),
        }
    }

    // Assign files to their closest sub-project
    if count > 0 {
        assign_files_to_subprojects(sqlite, &subprojects).await;
    }

    count
}

/// Resolve npm/pnpm/yarn workspace package directories from root manifests.
async fn resolve_npm_workspaces(root: &Path) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();

    // Try pnpm-workspace.yaml first
    let pnpm_ws = root.join("pnpm-workspace.yaml");
    if pnpm_ws.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&pnpm_ws).await {
            // Simple YAML parsing: extract lines matching "  - packages/*" patterns
            for line in content.lines() {
                let trimmed = line.trim().trim_start_matches('-').trim();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "packages:" {
                    continue;
                }
                let pattern = trimmed.trim_matches(|c| c == '\'' || c == '"');
                resolve_glob_pattern(root, pattern, &mut dirs);
            }
        }
        return dirs;
    }

    // Try package.json workspaces field
    let pkg_json = root.join("package.json");
    if pkg_json.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&pkg_json).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // "workspaces": ["packages/*", "apps/*"]
                // or "workspaces": { "packages": ["packages/*"] }
                let patterns = if let Some(arr) = json.get("workspaces").and_then(|w| w.as_array())
                {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                } else if let Some(obj) = json.get("workspaces").and_then(|w| w.as_object()) {
                    obj.get("packages")
                        .and_then(|p| p.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                for pattern in &patterns {
                    resolve_glob_pattern(root, pattern, &mut dirs);
                }
            }
        }
    }

    dirs
}

/// Resolve a workspace glob pattern (e.g. "packages/*") into actual directories.
fn resolve_glob_pattern(root: &Path, pattern: &str, dirs: &mut Vec<std::path::PathBuf>) {
    let full_pattern = root.join(pattern);
    let pattern_str = full_pattern.to_string_lossy();

    if let Ok(entries) = glob::glob(&pattern_str) {
        for entry in entries.flatten() {
            if entry.is_dir() && entry.join("package.json").exists() {
                dirs.push(entry);
            }
        }
    }
}

/// Assign existing files to their closest owning sub-project.
async fn assign_files_to_subprojects(
    sqlite: &SqliteStorage,
    subprojects: &[(String, String, String, Option<String>)],
) {
    // Sort by path length descending so deeper sub-projects match first
    let mut sorted: Vec<&(String, String, String, Option<String>)> = subprojects.iter().collect();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    // Get all subproject IDs
    let sps = match sqlite.list_subprojects().await {
        Ok(v) => v,
        Err(_) => return,
    };

    let sp_map: std::collections::HashMap<&str, i64> =
        sps.iter().map(|s| (s.path.as_str(), s.id)).collect();

    // Get all file hashes (reuse existing method that returns path->hash)
    let files = match sqlite.get_all_file_hashes().await {
        Ok(f) => f,
        Err(_) => return,
    };

    for file_path in files.keys() {
        // Find the deepest sub-project that is a prefix of this file's path
        for sp in &sorted {
            if file_path.starts_with(&sp.1) {
                if let Some(&sp_id) = sp_map.get(sp.1.as_str()) {
                    if let Ok(Some(file)) = sqlite.get_file(file_path).await {
                        let _ = sqlite.set_file_subproject(file.id, sp_id).await;
                    }
                }
                break;
            }
        }
    }
}
