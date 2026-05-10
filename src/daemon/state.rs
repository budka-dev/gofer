//! Daemon state — holds global resources and per-project state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

use super::registry::{ProjectRecord, RegistryDb};
use crate::cache::CacheManager;
use crate::error_recovery::CircuitBreaker; // Feature 016
use crate::indexer::{load_config, start_watcher, EmbedderPool, IndexTask, IndexerService};
use crate::languages::LanguageService;
use crate::resource_limits::ResourceLimits; // Feature 015
use crate::storage::{LanceStorage, SqliteStorage};

/// Global daemon state — lives for the lifetime of the daemon process.
pub struct DaemonState {
    /// Global home directory (~/.gofer/)
    pub gofer_home: PathBuf,
    /// Project registry
    pub registry: RegistryDb,
    /// Active projects keyed by project UUID
    pub projects: RwLock<HashMap<String, Arc<ProjectState>>>,
    /// Daemon start time
    pub started_at: Instant,
    /// Sync progress (shared with pipeline)
    pub sync_progress: Arc<SyncProgress>,
    /// Cancellation token for graceful shutdown
    pub shutdown_token: CancellationToken,
    /// Max concurrent connections semaphore
    pub connection_semaphore: Arc<Semaphore>,
    /// Runtime metrics (lock-free counters)
    pub metrics: Arc<DaemonMetrics>,
    /// Broadcast channel for server-to-client notifications (e.g. tools/list_changed)
    pub notify_tx: broadcast::Sender<String>,
    /// Resource limits for connection pooling and request throttling (Feature 015)
    pub resource_limits: Arc<ResourceLimits>,
    /// Circuit breaker for embedding API (Feature 016)
    pub embedding_circuit: Arc<CircuitBreaker>,
    /// Circuit breaker for vector search (Feature 016)
    pub vector_circuit: Arc<CircuitBreaker>,
    /// Language manager for tracking and resolving languages
    pub lang_manager: Arc<crate::indexer::parser::lang_manager::LanguageManager>,
    /// Security approvals for sandbox code execution (id -> (command, oneshot_sender))
    pub pending_confirmations:
        Arc<dashmap::DashMap<String, (String, tokio::sync::oneshot::Sender<bool>)>>,
}

/// Lock-free runtime metrics for the daemon process.
pub struct DaemonMetrics {
    /// Total files indexed (cumulative across all syncs)
    pub total_files_indexed: AtomicUsize,
    /// Total chunks embedded (cumulative)
    pub total_chunks_embedded: AtomicUsize,
    /// Total search queries served
    pub queries_served: AtomicUsize,
    /// Cumulative query latency in microseconds (divide by queries_served for avg)
    pub query_latency_us: AtomicUsize,
    /// Last full sync duration in milliseconds
    pub last_sync_duration_ms: AtomicUsize,
    /// Number of full syncs completed
    pub syncs_completed: AtomicUsize,
}

impl DaemonMetrics {
    pub fn new() -> Self {
        Self {
            total_files_indexed: AtomicUsize::new(0),
            total_chunks_embedded: AtomicUsize::new(0),
            queries_served: AtomicUsize::new(0),
            query_latency_us: AtomicUsize::new(0),
            last_sync_duration_ms: AtomicUsize::new(0),
            syncs_completed: AtomicUsize::new(0),
        }
    }

    pub fn record_query(&self, latency_us: usize) {
        self.queries_served.fetch_add(1, Ordering::Relaxed);
        self.query_latency_us
            .fetch_add(latency_us, Ordering::Relaxed);
    }

    pub fn record_sync(&self, files: usize, chunks: usize, duration_ms: usize) {
        self.total_files_indexed.fetch_add(files, Ordering::Relaxed);
        self.total_chunks_embedded
            .fetch_add(chunks, Ordering::Relaxed);
        self.last_sync_duration_ms
            .store(duration_ms, Ordering::Relaxed);
        self.syncs_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let queries = self.queries_served.load(Ordering::Relaxed);
        let latency_total = self.query_latency_us.load(Ordering::Relaxed);
        let avg_latency_us = if queries > 0 {
            latency_total / queries
        } else {
            0
        };

        serde_json::json!({
            "total_files_indexed": self.total_files_indexed.load(Ordering::Relaxed),
            "total_chunks_embedded": self.total_chunks_embedded.load(Ordering::Relaxed),
            "queries_served": queries,
            "avg_query_latency_us": avg_latency_us,
            "last_sync_duration_ms": self.last_sync_duration_ms.load(Ordering::Relaxed),
            "syncs_completed": self.syncs_completed.load(Ordering::Relaxed),
        })
    }

    /// Export metrics in Prometheus text exposition format.
    pub fn to_prometheus(&self) -> String {
        let queries = self.queries_served.load(Ordering::Relaxed);
        let latency_total = self.query_latency_us.load(Ordering::Relaxed);
        let avg_latency = if queries > 0 {
            latency_total / queries
        } else {
            0
        };

        format!(
            "# HELP gofer_files_indexed_total Total files indexed.\n\
             # TYPE gofer_files_indexed_total counter\n\
             gofer_files_indexed_total {}\n\
             # HELP gofer_chunks_embedded_total Total chunks embedded.\n\
             # TYPE gofer_chunks_embedded_total counter\n\
             gofer_chunks_embedded_total {}\n\
             # HELP gofer_queries_served_total Total search queries served.\n\
             # TYPE gofer_queries_served_total counter\n\
             gofer_queries_served_total {}\n\
             # HELP gofer_query_latency_avg_us Average query latency in microseconds.\n\
             # TYPE gofer_query_latency_avg_us gauge\n\
             gofer_query_latency_avg_us {}\n\
             # HELP gofer_last_sync_duration_ms Duration of last sync in milliseconds.\n\
             # TYPE gofer_last_sync_duration_ms gauge\n\
             gofer_last_sync_duration_ms {}\n\
             # HELP gofer_syncs_completed_total Total syncs completed.\n\
             # TYPE gofer_syncs_completed_total counter\n\
             gofer_syncs_completed_total {}\n",
            self.total_files_indexed.load(Ordering::Relaxed),
            self.total_chunks_embedded.load(Ordering::Relaxed),
            queries,
            avg_latency,
            self.last_sync_duration_ms.load(Ordering::Relaxed),
            self.syncs_completed.load(Ordering::Relaxed),
        )
    }
}

/// Progress tracking for sync pipeline — uses atomics for lock-free reads.
pub struct SyncProgress {
    pub active: AtomicBool,
    pub stage: Mutex<String>,
    pub files_total: AtomicUsize,
    pub files_scanned: AtomicUsize,
    pub files_parsed: AtomicUsize,
    pub chunks_embedded: AtomicUsize,
    pub files_written: AtomicUsize,
}

impl SyncProgress {
    pub fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            stage: Mutex::new(String::new()),
            files_total: AtomicUsize::new(0),
            files_scanned: AtomicUsize::new(0),
            files_parsed: AtomicUsize::new(0),
            chunks_embedded: AtomicUsize::new(0),
            files_written: AtomicUsize::new(0),
        }
    }

    pub fn reset(&self) {
        self.active.store(true, Ordering::Relaxed);
        self.files_total.store(0, Ordering::Relaxed);
        self.files_scanned.store(0, Ordering::Relaxed);
        self.files_parsed.store(0, Ordering::Relaxed);
        self.chunks_embedded.store(0, Ordering::Relaxed);
        self.files_written.store(0, Ordering::Relaxed);
    }

    pub fn finish(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> SyncProgressSnapshot {
        SyncProgressSnapshot {
            active: self.active.load(Ordering::Relaxed),
            files_total: self.files_total.load(Ordering::Relaxed),
            files_scanned: self.files_scanned.load(Ordering::Relaxed),
            files_parsed: self.files_parsed.load(Ordering::Relaxed),
            chunks_embedded: self.chunks_embedded.load(Ordering::Relaxed),
            files_written: self.files_written.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot for serialization.
pub struct SyncProgressSnapshot {
    pub active: bool,
    pub files_total: usize,
    pub files_scanned: usize,
    pub files_parsed: usize,
    pub chunks_embedded: usize,
    pub files_written: usize,
}

/// Per-project resources loaded into daemon memory.
pub struct ProjectState {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub sqlite: SqliteStorage,
    pub lance: Arc<LanceStorage>,
    pub embedder: Arc<EmbedderPool>,
    pub task_tx: mpsc::Sender<Vec<IndexTask>>,
    pub language_services: Arc<Vec<Box<dyn LanguageService>>>,
    /// Whether a file watcher is active
    pub watcher_active: Mutex<bool>,
    /// Cancellation token for stopping this project's background tasks
    pub cancel: CancellationToken,
    /// Cache manager for this project
    pub cache: Arc<CacheManager>,
    /// active LSP clients for this project (lazy-loaded per language)
    pub lsp_clients:
        Arc<RwLock<HashMap<String, Arc<crate::languages::generic_lsp::GenericLspClient>>>>,
}

impl DaemonState {
    /// Create a new DaemonState, loading the global embedder and reranker.
    pub async fn new(gofer_home: PathBuf) -> Result<Self> {
        let registry_path = gofer_home.join("registry.sqlite");
        let registry_path_str = registry_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid registry path: non-UTF8 characters"))?;
        let registry = RegistryDb::new(registry_path_str).await?;

        let (notify_tx, _) = broadcast::channel::<String>(64);

        // Feature 016: Circuit breakers for external services
        // Embedding API: 5 failures, 2 successes to recover, 30s timeout
        let embedding_circuit = Arc::new(CircuitBreaker::new(
            5,
            2,
            std::time::Duration::from_secs(30),
        ));

        // Vector search: 3 failures, 1 success to recover, 10s timeout
        let vector_circuit = Arc::new(CircuitBreaker::new(
            3,
            1,
            std::time::Duration::from_secs(10),
        ));

        Ok(Self {
            gofer_home: gofer_home.clone(),
            registry,
            projects: RwLock::new(HashMap::new()),
            started_at: Instant::now(),
            sync_progress: Arc::new(SyncProgress::new()),
            shutdown_token: CancellationToken::new(),
            connection_semaphore: Arc::new(Semaphore::new(1024)),
            metrics: Arc::new(DaemonMetrics::new()),
            notify_tx,
            resource_limits: Arc::new(ResourceLimits::default()), // Feature 015
            embedding_circuit,                                    // Feature 016
            vector_circuit,                                       // Feature 016
            lang_manager: Arc::new(
                crate::indexer::parser::lang_manager::LanguageManager::new(
                    Some(gofer_home.clone().join("langs")),
                    Some(gofer_home.clone().join("tools")),
                )
                .unwrap_or_else(|_| {
                    crate::indexer::parser::lang_manager::LanguageManager::new(None, None).unwrap()
                }),
            ),
            pending_confirmations: Arc::new(dashmap::DashMap::new()),
        })
    }

    /// Ask the user for permission to execute a shell command.
    /// Blocks until the user answers (via TUI) or 1 hour timeout.
    pub async fn request_confirmation(&self, command: &str) -> bool {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id = uuid::Uuid::new_v4().to_string();
        self.pending_confirmations
            .insert(id.clone(), (command.to_string(), tx));

        match tokio::time::timeout(std::time::Duration::from_secs(3600), rx).await {
            Ok(Ok(approved)) => approved,
            _ => {
                self.pending_confirmations.remove(&id);
                false
            }
        }
    }

    /// Resolve a project path to a loaded ProjectState, loading it on demand.
    pub async fn get_or_load_project(&self, project_path: &str) -> Result<Arc<ProjectState>> {
        // Fast path: already loaded
        {
            let projects = self.projects.read().await;
            for ps in projects.values() {
                if ps.path == Path::new(project_path) {
                    return Ok(ps.clone());
                }
            }
        }

        // Must be registered
        let record = self
            .registry
            .get_by_path(project_path)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Project not registered: {}. Run 'gofer init' in the project directory first.",
                    project_path
                )
            })?;

        self.registry.update_last_opened(&record.id).await?;
        self.load_project(&record).await
    }

    /// Load a project into memory given its registry record.
    async fn load_project(&self, record: &ProjectRecord) -> Result<Arc<ProjectState>> {
        let index_dir = self.gofer_home.join("indices").join(&record.id);
        tokio::fs::create_dir_all(&index_dir).await?;

        // Load config from project root to respect settings (e.g. parallel_workers)
        let root_path = PathBuf::from(&record.path);
        let gofer_dir = root_path.join(".gofer");
        let config = load_config(&gofer_dir);
        let workers = config.indexer.parallel_workers.unwrap_or(4);
        let embedder = Arc::new(EmbedderPool::with_config(
            config.embedding.pool_size,
            &config.embedding,
        )?);

        let db_path = index_dir.join("graph.db");
        let lance_path = index_dir.join("lancedb");

        let db_path_str = db_path.to_str().ok_or_else(|| {
            anyhow::anyhow!("Invalid SQLite path: non-UTF8 characters in {:?}", db_path)
        })?;
        let sqlite = SqliteStorage::new(db_path_str).await?;
        sqlite.migrate().await?;

        // Run integrity check on SQLite database
        if let Err(e) = sqlite.check_integrity().await {
            tracing::error!(
                "SQLite integrity check failed for project {}: {}",
                record.id,
                e
            );
            // Continue anyway - the database may still be usable
        }

        // C3: Invalidate chunk embedding cache if the model changed since last index
        let cache_version_key = "embedding_cache_version";
        let current_version = embedder.cache_version_key();
        match sqlite.get_index_meta(cache_version_key).await? {
            Some(stored) if stored == current_version => {}
            Some(old_version) => {
                tracing::warn!(
                    "Embedding model changed ({} → {}), clearing chunk cache",
                    old_version,
                    current_version
                );
                sqlite.clear_chunk_cache().await?;
                sqlite.reset_all_indexing_status().await?;
                sqlite
                    .set_index_meta(cache_version_key, &current_version)
                    .await?;
                    
                if lance_path.exists() {
                    tracing::info!("Dropping LanceDB to recreate with new dimensions");
                    tokio::fs::remove_dir_all(&lance_path).await.ok();
                }
            }
            None => {
                sqlite
                    .set_index_meta(cache_version_key, &current_version)
                    .await?;
            }
        }

        let lance_path_str = lance_path.to_str().ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid LanceDB path: non-UTF8 characters in {:?}",
                lance_path
            )
        })?;
        let lance_storage = LanceStorage::new(lance_path_str, embedder.dimension()).await?;

        // Run health check on LanceDB
        if let Err(e) = lance_storage.health_check().await {
            tracing::error!(
                "LanceDB health check failed for project {}: {}",
                record.id,
                e
            );
            // Continue anyway - the database may still be usable
        }

        let lance = Arc::new(lance_storage);

        let (task_tx, task_rx) = mpsc::channel::<Vec<IndexTask>>(100);

        let language_services = Arc::new(init_language_services(&sqlite, &root_path));

        let project_cancel = CancellationToken::new();

        // Create cache manager for this project
        let cache = Arc::new(CacheManager::new());

        let state = Arc::new(ProjectState {
            id: record.id.clone(),
            path: root_path.clone(),
            name: record.name.clone(),
            sqlite: sqlite.clone(),
            lance: lance.clone(),
            embedder: embedder.clone(),
            task_tx,
            language_services,
            watcher_active: Mutex::new(false),
            cancel: project_cancel,
            cache: cache.clone(),
            lsp_clients: Arc::new(RwLock::new(HashMap::new())),
        });

        // Spawn indexer worker — shares lance + embedder pool via Arc
        // Feature 012: Pass cache for invalidation on file changes
        // Use configured parallel workers
        let indexer =
            IndexerService::new(sqlite, lance, embedder.clone(), workers).with_cache(cache);

        tokio::spawn(async move {
            indexer.run(task_rx).await;
        });

        // Store in map
        let mut projects = self.projects.write().await;
        projects.insert(record.id.clone(), state.clone());

        tracing::info!("Loaded project: {} ({})", record.name, record.path);

        // Notify connected clients that the tool list may have changed
        if !state.language_services.is_empty() {
            let notif = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/tools/list_changed"
            });
            let _ = self.notify_tx.send(notif.to_string());
        }

        Ok(state)
    }

    /// Activate a project: load + full_sync + start watcher.
    pub async fn activate_project(
        &self,
        project_path: &str,
        watch: bool,
        background: bool,
    ) -> Result<String> {
        let record = self
            .registry
            .get_by_path(project_path)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Project not registered: {}", project_path))?;

        let project = self.get_or_load_project(project_path).await?;

        // Check if we can skip full sync because it's already running and being watched
        let mut should_start_watcher = false;
        if watch {
            let mut watcher_active = project.watcher_active.lock().await;
            if *watcher_active {
                tracing::info!(
                    "Project '{}' is already actively watched. Skipping redundant full_sync.",
                    record.name
                );
                return Ok(format!(
                    "Project '{}' activated (already watching)",
                    record.name
                ));
            } else {
                // Mark as active immediately to prevent concurrent activate_project calls
                // from spawning redundant syncs and watchers.
                *watcher_active = true;
                should_start_watcher = true;
            }
        }

        // Run full sync
        let index_dir = self.gofer_home.join("indices").join(&record.id);

        // Load config from project dir (or index dir)
        let config_path = index_dir.join("config.toml");
        let gofer_dir = if config_path.exists() {
            index_dir.clone()
        } else {
            // Fallback: check project root for .gofer/config.toml
            PathBuf::from(project_path).join(".gofer")
        };
        let config = load_config(&gofer_dir);
        let mut ignore_patterns = config.indexer.ignore.clone();
        for lang_entry in self.lang_manager.loaded_langs.iter() {
            if let Some(indexer) = &lang_entry.value().manifest.indexer {
                ignore_patterns.extend(indexer.ignore_folders.clone());
            }
        }
        let workers = config.indexer.parallel_workers.unwrap_or(4);

        // Full sync using shared lance + embedder pool (no redundant instances)
        // Note: parallel_workers here is for consistency, full_sync uses internal pipeline
        let sync_indexer = IndexerService::new(
            project.sqlite.clone(),
            project.lance.clone(),
            project.embedder.clone(),
            workers,
        );

        let root = PathBuf::from(project_path);

        if background {
            let progress_clone = self.sync_progress.clone();
            let metrics_clone = self.metrics.clone();
            let cancel_clone = project.cancel.clone();
            let root_clone = root.clone();
            let ignore_clone = ignore_patterns.clone();
            let sync_indexer_clone = sync_indexer.clone();

            tokio::spawn(async move {
                if let Err(e) = sync_indexer_clone
                    .full_sync(
                        &root_clone,
                        &ignore_clone,
                        Some(progress_clone),
                        Some(metrics_clone),
                        cancel_clone,
                    )
                    .await
                {
                    tracing::error!("Background full sync failed: {}", e);
                }
            });
        } else {
            sync_indexer
                .full_sync(
                    &root,
                    &ignore_patterns,
                    Some(self.sync_progress.clone()),
                    Some(self.metrics.clone()),
                    project.cancel.clone(),
                )
                .await?;
        }

        // Start watcher if requested
        if should_start_watcher {
            let watcher_root = root.clone();
            let watcher_tx = project.task_tx.clone();
            let watcher_ignores = ignore_patterns;
            let watcher_cancel = project.cancel.clone();
            tokio::spawn(async move {
                start_watcher(watcher_root, watcher_tx, watcher_ignores, watcher_cancel).await;
            });
            tracing::info!("Watcher started for {}", project_path);
        }

        Ok(format!(
            "Project '{}' activated{}",
            record.name,
            if watch { " with watcher" } else { "" }
        ))
    }

    /// Deactivate a project: stop watcher, remove from memory.
    pub async fn deactivate_project(&self, project_path: &str) -> Result<()> {
        let mut projects = self.projects.write().await;
        let id_to_remove = projects
            .iter()
            .find(|(_, ps)| ps.path == Path::new(project_path))
            .map(|(id, _)| id.clone());

        if let Some(id) = id_to_remove {
            if let Some(ps) = projects.remove(&id) {
                // Stop LSP servers
                let mut clients = ps.lsp_clients.write().await;
                for (_, client) in clients.drain() {
                    let _ = client.stop().await;
                }
                ps.cancel.cancel();
            }
            tracing::info!("Deactivated project: {}", project_path);
        }
        Ok(())
    }

    /// Get or start LSP client instance for a given file
    #[allow(dead_code)]
    pub async fn get_lsp_client(
        &self,
        project_path: &str,
        file_path: &str,
    ) -> Result<Option<Arc<crate::languages::generic_lsp::GenericLspClient>>> {
        let project = self.get_or_load_project(project_path).await?;
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // 1. Resolve Language via lang_manager
        let lang_name = match self.lang_manager.get_language_by_ext(ext) {
            Some(l) => l,
            None => return Ok(None), // No language support
        };

        // 2. Load manifest to find LSP details
        let loaded_lang = match self.lang_manager.get_language(&lang_name) {
            Some(l) => l,
            None => return Ok(None),
        };

        let lsp_config = match &loaded_lang.manifest.lsp {
            Some(config) => config.clone(),
            None => return Ok(None), // No LSP configured for this language
        };

        let lang_id = lsp_config.name.clone().unwrap_or_else(|| lang_name.clone());

        // Fast path: already initialized
        {
            let clients_guard = project.lsp_clients.read().await;
            if let Some(client) = clients_guard.get(&lang_id) {
                if client.is_ready().await {
                    return Ok(Some(client.clone()));
                }
            }
        }

        // Slow path: initialize LSP server
        let mut clients_guard = project.lsp_clients.write().await;

        // Double-check in case another task initialized it while we were waiting
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
                        let local_exe = self
                            .lang_manager
                            .tools_dir
                            .join(t)
                            .join("bin")
                            .join(exe_name);

                        if !local_exe.exists() {
                            tracing::info!(
                                "Tool executable {} not found locally. Attempting to download...",
                                exe_name
                            );
                            if let Ok(path) = self
                                .lang_manager
                                .download_and_extract_binary(t, &tool_manifest)
                                .await
                            {
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
        let shell_args =
            shell_words::split(&command_str).unwrap_or_else(|_| vec![command_str.clone()]);
        let cmd = shell_args[0].clone();
        let mut args: Vec<String> = shell_args.into_iter().skip(1).collect();
        args.extend(tool_args);

        // Probe the LSP binary up-front. Spawning a server only to have the
        // process fail to exec leaves a confusing "Failed to spawn LSP server"
        // error for the user; checking PATH first lets us return an actionable
        // install hint instead.
        if !lsp_binary_available(&cmd).await {
            let hint = lsp_install_hint(&cmd, &lang_id);
            return Err(anyhow::anyhow!(
                "LSP server `{}` not found in PATH (needed for {} tools). {}",
                cmd,
                lang_id,
                hint
            ));
        }

        let root_path = project.path.clone();
        let mut actual_root = root_path.clone();
        if let Ok(file_path_buf) =
            crate::daemon::handlers::common::resolve_path_buf(&root_path, file_path)
        {
            actual_root = crate::daemon::handlers::common::find_project_root(
                &file_path_buf,
                &loaded_lang.manifest.language.root_markers,
                &root_path,
            );
        }

        let init_options = lsp_config.init_options.clone();

        let client = Arc::new(crate::languages::generic_lsp::GenericLspClient::new(
            actual_root,
            cmd,
            args,
            lang_id.clone(),
            init_options,
        ));

        client.start().await?;
        clients_guard.insert(lang_id.clone(), client.clone());

        Ok(Some(client))
    }
}

/// Check whether `cmd` resolves to an executable file. Handles both absolute
/// paths (from manifest `command`) and bare binary names (resolved via PATH).
async fn lsp_binary_available(cmd: &str) -> bool {
    if cmd.contains('/') {
        return tokio::fs::metadata(cmd)
            .await
            .map(|m| m.is_file())
            .unwrap_or(false);
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_var) {
        if dir.join(cmd).is_file() {
            return true;
        }
    }
    false
}

/// Map known LSP binaries to their install commands. Falls back to a generic
/// "install <cmd>" message for unknown servers.
fn lsp_install_hint(cmd: &str, lang_id: &str) -> String {
    let hint = match cmd {
        "typescript-language-server" => Some(
            "Install: `npm i -g typescript-language-server typescript` or `bun add -g typescript-language-server typescript`."
        ),
        "rust-analyzer" => Some(
            "Install: `rustup component add rust-analyzer` or download from https://rust-analyzer.github.io"
        ),
        "pyright" | "pyright-langserver" => Some(
            "Install: `npm i -g pyright`."
        ),
        "gopls" => Some(
            "Install: `go install golang.org/x/tools/gopls@latest`."
        ),
        "vscode-eslint-language-server" => Some(
            "Install: `npm i -g vscode-langservers-extracted`."
        ),
        "vue-language-server" | "volar" => Some(
            "Install: `npm i -g @vue/language-server`."
        ),
        _ => None,
    };
    match hint {
        Some(h) => h.to_string(),
        None => format!("Install the {} language server and ensure it's on PATH.", lang_id),
    }
}

/// Initialize language services for a project.
fn init_language_services(
    sqlite: &SqliteStorage,
    root_path: &Path,
) -> Vec<Box<dyn LanguageService>> {
    let mut services: Vec<Box<dyn LanguageService>> = Vec::new();

    let rust_svc = crate::languages::rust::RustService::new(sqlite.clone());
    if rust_svc.is_applicable(root_path) {
        services.push(Box::new(rust_svc));
    }

    let vue_svc = crate::languages::vue::VueService::new(sqlite.clone());
    if vue_svc.is_applicable(root_path) {
        services.push(Box::new(vue_svc));
    }

    let ts_svc = crate::languages::typescript::TypeScriptService::new(sqlite.clone(), root_path);
    if ts_svc.is_applicable(root_path) {
        services.push(Box::new(ts_svc));
    }

    let py_svc = crate::languages::python::PythonService::new(sqlite.clone(), root_path);
    if py_svc.is_applicable(root_path) {
        services.push(Box::new(py_svc));
    }

    let go_svc = crate::languages::go::GoService::new(sqlite.clone());
    if go_svc.is_applicable(root_path) {
        services.push(Box::new(go_svc));
    }

    services
}
