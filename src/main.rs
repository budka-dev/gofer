#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod cache; // Feature 008: server-side LRU cache
mod commit;
mod daemon;
mod error;
mod error_recovery; // Feature 016: graceful error handling & recovery
mod indexer;
mod ipc;
mod languages;
mod logger;
mod models;
mod resource_limits; // Feature 015: connection pooling & resource management
mod scoring_index;
mod storage; // rkyv-based hot index for file scoring
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde_json::json;

use ipc::client::DaemonClient;

#[derive(Parser)]
#[command(name = "gofer")]
#[command(about = "Project Memory Service for Qoder", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Log level filter (e.g. debug, info, warn, error)
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon (if needed) and register current project
    Hi,

    /// Register current project with the daemon
    Init,

    /// Activate project in the daemon (full sync + optional watcher)
    Start {
        /// Enable file watching
        #[arg(long, default_value_t = true)]
        watch: bool,
    },

    /// Run as MCP server (stdio bridge to daemon)
    Mcp {
        /// Project directory path (defaults to current working directory)
        #[arg(long)]
        project_dir: Option<String>,
    },

    /// Show daemon and project status
    Status,

    /// Check daemon health
    Health,

    /// Search the codebase via daemon
    Search {
        /// Search query
        query: String,
        /// Maximum number of results
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },

    /// Trigger re-indexing via daemon
    Reindex {
        /// Full reset: clear all data and reindex
        #[arg(short, long)]
        force: bool,
        /// Reindex specific file only
        #[arg(long)]
        path: Option<String>,
    },

    /// Start daemon, register, activate, and stream file watcher events
    Watch,

    /// Shutdown the daemon
    Stop,

    /// View daemon logs
    Logs {
        /// Number of lines to show (from end)
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,
        /// Show error log instead of main log
        #[arg(long)]
        err: bool,
    },

    /// Manage project configuration (.gofer/config.toml)
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Install language parser from GitHub lang-hub
    InstallLang {
        /// Name of the language to install (e.g. rust, vue)
        lang: String,
    },

    /// (internal) Run as daemon process
    #[command(hide = true)]
    Daemon,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create default config.toml in .gofer/ directory
    Init,
    /// Show path to the config file
    Path,
}

fn gofer_home() -> PathBuf {
    dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".gofer")
}

fn socket_path() -> PathBuf {
    gofer_home().join("daemon.sock")
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logger for CLI and MCP processes (Daemon initializes after fork)
    let _logger_guard = match &cli.command {
        Commands::Daemon => None,
        Commands::Mcp { .. } => Some(crate::logger::init("mcp")),
        _ => Some(crate::logger::init("cli")),
    };

    match cli.command {
        Commands::Daemon => run_daemon(),
        Commands::Hi => handle_hi(),
        Commands::Init => handle_init(),
        Commands::Start { watch } => handle_start(watch),
        Commands::Mcp { project_dir } => handle_mcp(project_dir),
        Commands::Status => handle_status(),
        Commands::Health => handle_health(),
        Commands::Reindex { force, path } => handle_reindex(force, path),
        Commands::Search { query, limit } => handle_search(&query, limit),
        Commands::Watch => handle_watch(),
        Commands::Stop => handle_stop(),
        Commands::Logs { lines, follow, err } => handle_logs(lines, follow, err),
        Commands::Config { action } => handle_config(action),
        Commands::InstallLang { lang } => handle_install_lang(lang),
    }
}

// === Daemon entry point ===

fn run_daemon() -> anyhow::Result<()> {
    let home = gofer_home();
    std::fs::create_dir_all(&home)?;

    let log_file = home.join("daemon.log");
    let err_file = home.join("daemon.err");
    let pid_file = home.join("daemon.pid");

    let stdout_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&err_file)?;

    let daemonize = daemonize::Daemonize::new()
        .pid_file(&pid_file)
        .stdout(stdout_file)
        .stderr(stderr_file);

    match daemonize.start() {
        Ok(_) => {
            // We are now the daemon process
            // Optimize tokio runtime to reduce thread explosion
            let num_cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);

            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads((num_cpus / 2).max(4)) // Use 50% of CPUs for async tasks, min 4
                .max_blocking_threads(num_cpus) // Limit blocking thread pool to num_cpus
                .thread_name("gofer-worker")
                .enable_all()
                .build()?;

            tracing::info!(
                "Tokio runtime configured: {} worker threads, {} max blocking threads",
                (num_cpus / 2).max(4),
                num_cpus
            );

            rt.block_on(async {
                let _logger_guard = crate::logger::init("daemon");

                tracing::info!("gofer daemon starting (pid {})", std::process::id());
                let state = Arc::new(daemon::state::DaemonState::new(home).await?);

                // Start Prometheus metrics HTTP server
                let metrics_state = state.metrics.clone();
                let metrics_cancel = state.shutdown_token.clone();
                tokio::spawn(async move {
                    daemon::metrics_http::serve_metrics(
                        "127.0.0.1:9091",
                        metrics_state,
                        metrics_cancel,
                    )
                    .await;
                });

                // Register signal handlers to trigger graceful shutdown
                let token = state.shutdown_token.clone();
                tokio::spawn(async move {
                    let mut sigterm =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                            .expect("failed to register SIGTERM handler");
                    let mut sigint =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                            .expect("failed to register SIGINT handler");

                    tokio::select! {
                        _ = sigterm.recv() => {
                            tracing::info!("SIGTERM received, initiating graceful shutdown");
                        }
                        _ = sigint.recv() => {
                            tracing::info!("SIGINT received, initiating graceful shutdown");
                        }
                    }
                    token.cancel();
                });

                ipc::server::run_daemon(state).await
            })
        }
        Err(e) => {
            eprintln!("Failed to daemonize: {}", e);
            std::process::exit(1);
        }
    }
}

// === Shared helper ===

/// Ensure the daemon is running, auto-starting it if necessary.
async fn ensure_daemon_running() -> anyhow::Result<()> {
    let sock = socket_path();

    if DaemonClient::is_alive(&sock).await {
        return Ok(());
    }

    // Launch daemon process
    let exe = std::env::current_exe()?;

    tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    })
    .await??;

    // Wait for socket (up to 30 seconds — model loading may be slow)
    let sock = socket_path();
    for _ in 0..300 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if DaemonClient::is_alive(&sock).await {
            return Ok(());
        }
    }

    anyhow::bail!("Daemon failed to start within 30 seconds (check ~/.gofer/daemon.err)")
}

/// Helper: build a multi-threaded tokio runtime for CLI commands.
fn cli_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Runtime::new()?)
}

// === CLI handlers ===

/// Activate a project with real-time progress display.
/// Spawns the activate call in a task and polls daemon/sync_progress.
async fn activate_with_progress(
    sock: &std::path::Path,
    project_path: &str,
    watch: bool,
) -> anyhow::Result<()> {
    use std::io::Write;

    let sock_owned = sock.to_path_buf();
    let path_owned = project_path.to_string();

    let sock_spawn = sock_owned.clone();
    let path_spawn = path_owned.clone();

    // Spawn activation in background task
    let activate_handle = tokio::spawn(async move {
        let mut client = DaemonClient::connect(&sock_spawn).await?;
        client
            .call(
                "daemon/activate_project",
                json!({ "project_path": path_spawn, "watch": watch }),
            )
            .await
    });

    // Poll progress every 250ms until activation completes
    let mut last_line_len = 0usize;
    let mut last_written = 0u64;
    let mut stall_ticks = 0u32;
    const STALL_TIMEOUT_TICKS: u32 = 120; // 30 seconds (120 * 250ms)
    const ABSOLUTE_TIMEOUT_TICKS: u32 = 240; // 60 seconds total
    let mut total_ticks = 0u32;

    // Setup signal handler for graceful interruption
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok();

    loop {
        tokio::select! {
            _ = async {
                if let Some(ref mut s) = sigint {
                    s.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                eprintln!("\nActivation cancelled by user. Stopping background indexing...");
                if let Ok(mut client) = DaemonClient::connect(&sock_owned).await {
                    let _ = client.call("daemon/deactivate_project", json!({ "project_path": path_owned.clone() })).await;
                }
                break;
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
        }

        total_ticks += 1;

        // Check if activation finished
        if activate_handle.is_finished() {
            break;
        }

        // Absolute timeout check
        if total_ticks > ABSOLUTE_TIMEOUT_TICKS {
            eprintln!("\nWarning: Activation timeout after 60 seconds");
            eprintln!("Check daemon logs: gofer logs -n 100");
            break;
        }

        // Poll progress
        if let Ok(mut poll_client) = DaemonClient::connect(sock).await {
            if let Ok(prog) = poll_client.call("daemon/sync_progress", json!({})).await {
                let active = prog
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !active {
                    continue;
                }
                let total = prog
                    .get("files_total")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let scanned = prog
                    .get("files_scanned")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let parsed = prog
                    .get("files_parsed")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let embedded = prog
                    .get("chunks_embedded")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let written = prog
                    .get("files_written")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                // Stall detection: no progress in write counter
                if written == last_written && scanned > 0 {
                    stall_ticks += 1;
                    if stall_ticks >= STALL_TIMEOUT_TICKS {
                        // Check if all files already indexed (no work needed)
                        if scanned == total && written == 0 && parsed == 0 {
                            eprintln!("\nAll files already up-to-date, no indexing needed");
                            break;
                        } else {
                            eprintln!("\nWarning: Indexing stalled for 30 seconds");
                            eprintln!(
                                "Progress: scan:{} parse:{} embed:{} write:{}/{}",
                                scanned, parsed, embedded, written, total
                            );
                            eprintln!("Check daemon logs: gofer logs -n 100");
                            break;
                        }
                    }
                } else {
                    stall_ticks = 0;
                }
                last_written = written;

                let pct = if total > 0 {
                    ((written as f64 / total as f64) * 100.0).min(100.0) as u64
                } else if scanned > 0 {
                    // During scanning phase, show a small progress indicator
                    10
                } else {
                    0
                };

                // Build progress bar
                let bar_width = 30;
                let filled = (bar_width as u64 * pct / 100) as usize;
                let empty = bar_width - filled;
                let bar = format!(
                    "[{}{}] {}%  scan:{} parse:{} embed:{} write:{}/{}",
                    "#".repeat(filled),
                    "-".repeat(empty),
                    pct,
                    scanned,
                    parsed,
                    embedded,
                    written,
                    total,
                );

                // Clear previous line and print
                let padding = if bar.len() < last_line_len {
                    " ".repeat(last_line_len - bar.len())
                } else {
                    String::new()
                };
                print!("\r{}{}", bar, padding);
                let _ = std::io::stdout().flush();
                last_line_len = bar.len();
            }
        }
    }

    // Clear progress line
    if last_line_len > 0 {
        print!("\r{}\r", " ".repeat(last_line_len));
        let _ = std::io::stdout().flush();
    }

    // Get result from the activation task
    let result = activate_handle.await??;
    let msg = result
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Activated");
    println!("{}", msg);

    Ok(())
}

fn handle_hi() -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        println!("Starting daemon...");
        ensure_daemon_running().await?;

        let cwd = std::env::current_dir()?.canonicalize()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let sock = socket_path();

        let mut client = DaemonClient::connect(&sock).await?;

        // Register project
        let reg = client
            .call(
                "daemon/register_project",
                json!({ "project_path": cwd_str }),
            )
            .await?;
        let project_id = reg.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        println!("Registered: {} ({})", cwd_str, project_id);

        anyhow::Ok(())
    })
}

fn handle_init() -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        ensure_daemon_running().await?;

        let cwd = std::env::current_dir()?.canonicalize()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let sock = socket_path();

        let mut client = DaemonClient::connect(&sock).await?;
        let result = client
            .call(
                "daemon/register_project",
                json!({ "project_path": cwd_str }),
            )
            .await?;
        let project_id = result.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        println!("Project registered: {} (id: {})", cwd_str, project_id);

        anyhow::Ok(())
    })
}

fn handle_start(watch: bool) -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        ensure_daemon_running().await?;

        let cwd = std::env::current_dir()?.canonicalize()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let sock = socket_path();

        activate_with_progress(&sock, &cwd_str, watch).await?;

        anyhow::Ok(())
    })
}

fn handle_mcp(project_dir: Option<String>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        ensure_daemon_running().await?;

        let cwd = match project_dir {
            Some(dir) => std::path::PathBuf::from(dir).canonicalize()?,
            None => std::env::current_dir()?.canonicalize()?,
        };
        let sock = socket_path();
        ipc::bridge::run_bridge(cwd, &sock).await
    })
}

fn handle_status() -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        let sock = socket_path();

        if !DaemonClient::is_alive(&sock).await {
            println!("Status: STOPPED (daemon not running)");
            return anyhow::Ok(());
        }

        let mut client = DaemonClient::connect(&sock).await?;
        let result = client.call("daemon/status", json!({})).await?;

        println!("Status: RUNNING");

        if let Some(uptime) = result.get("uptime_seconds").and_then(|v| v.as_u64()) {
            let hours = uptime / 3600;
            let mins = (uptime % 3600) / 60;
            let secs = uptime % 60;
            println!("Uptime: {}h {}m {}s", hours, mins, secs);
        }

        if let Some(registered) = result.get("registered_projects").and_then(|v| v.as_u64()) {
            println!("Registered projects: {}", registered);
        }

        if let Some(active) = result.get("active_projects").and_then(|v| v.as_array()) {
            println!("Active projects: {}", active.len());
            for p in active {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let path = p.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  - {} ({})", name, path);
            }
        }

        anyhow::Ok(())
    })
}

fn handle_health() -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        let sock = socket_path();

        if !DaemonClient::is_alive(&sock).await {
            println!("{{\"status\":\"stopped\"}}");
            std::process::exit(1);
        }

        let mut client = DaemonClient::connect(&sock).await?;
        let result = client.call("daemon/health", json!({})).await?;
        println!("{}", serde_json::to_string_pretty(&result)?);

        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if status != "healthy" {
            std::process::exit(1);
        }

        anyhow::Ok(())
    })
}

fn handle_reindex(force: bool, path: Option<String>) -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        ensure_daemon_running().await?;

        let cwd = std::env::current_dir()?.canonicalize()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let sock = socket_path();

        let mut client = DaemonClient::connect(&sock).await?;

        if force {
            println!("Принудительная переиндексация: очистка данных...");
            let _ = client
                .call(
                    "reindex",
                    json!({
                        "project_path": cwd_str,
                        "force": true,
                        "path": serde_json::Value::Null,
                    }),
                )
                .await;
        } else if let Some(ref file_path) = path {
            let abs_path = cwd.join(file_path);
            let abs_str = abs_path.to_string_lossy().to_string();
            println!("Переиндексация файла: {}", abs_str);
            let _ = client
                .call(
                    "reindex",
                    json!({
                        "project_path": cwd_str,
                        "force": false,
                        "path": abs_str,
                    }),
                )
                .await;
        } else {
            // Стандартная переиндексация (incremental)
            activate_with_progress(&sock, &cwd_str, false).await?;
            return anyhow::Ok(());
        }

        // Для force и path — запустить sync с прогрессом
        activate_with_progress(&sock, &cwd_str, force).await?;

        anyhow::Ok(())
    })
}

fn handle_search(query: &str, limit: usize) -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    let query = query.to_string();
    rt.block_on(async {
        ensure_daemon_running().await?;

        let cwd = std::env::current_dir()?.canonicalize()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let sock = socket_path();

        let mut client = DaemonClient::connect(&sock).await?;

        let result = client
            .call(
                "tools/call",
                json!({
                    "project_path": cwd_str,
                    "name": "search",
                    "arguments": {
                        "query": query,
                        "limit": limit,
                        "mode": "hybrid",
                        "rerank": true,
                    }
                }),
            )
            .await?;

        if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
            for item in content {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    println!("{}", text);
                }
            }
        }

        anyhow::Ok(())
    })
}

fn handle_watch() -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        println!("Starting daemon...");
        ensure_daemon_running().await?;

        let cwd = std::env::current_dir()?.canonicalize()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let sock = socket_path();

        let mut client = DaemonClient::connect(&sock).await?;

        // Register
        let _ = client
            .call(
                "daemon/register_project",
                json!({ "project_path": cwd_str }),
            )
            .await?;

        // Activate with watcher + progress
        activate_with_progress(&sock, &cwd_str, true).await?;

        println!("Watching for changes... (press Ctrl+C to stop)");

        // Keep alive — poll status periodically
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if !DaemonClient::is_alive(&sock).await {
                println!("Daemon stopped.");
                break;
            }
        }

        anyhow::Ok(())
    })
}

fn handle_stop() -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        let sock = socket_path();

        if !DaemonClient::is_alive(&sock).await {
            println!("Daemon is not running.");
            return anyhow::Ok(());
        }

        let mut client = DaemonClient::connect(&sock).await?;
        let _ = client.call("daemon/shutdown", json!({})).await;
        println!("Daemon shutdown requested.");

        anyhow::Ok(())
    })
}

fn handle_logs(lines: usize, follow: bool, err: bool) -> anyhow::Result<()> {
    use std::io::{BufRead, BufReader, Seek};

    let home = gofer_home();
    let log_path = if err {
        home.join("daemon.err")
    } else {
        home.join("daemon.log")
    };

    if !log_path.exists() {
        eprintln!("Log file not found: {}", log_path.display());
        return Ok(());
    }

    let file = std::fs::File::open(&log_path)?;
    let reader = BufReader::new(&file);

    // Read all lines and show last N
    let all_lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;
    let start = all_lines.len().saturating_sub(lines);
    for line in &all_lines[start..] {
        println!("{}", line);
    }

    if !follow {
        return Ok(());
    }

    // Follow mode: seek to end and poll for new data
    let mut file = std::fs::File::open(&log_path)?;
    file.seek(std::io::SeekFrom::End(0))?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf = String::new();

    loop {
        buf.clear();
        match std::io::BufRead::read_line(&mut reader, &mut buf) {
            Ok(0) => {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Ok(_) => {
                print!("{}", buf);
            }
            Err(e) => {
                eprintln!("Error reading log: {}", e);
                break;
            }
        }
    }

    Ok(())
}

fn handle_install_lang(lang: String) -> anyhow::Result<()> {
    let rt = cli_runtime()?;
    rt.block_on(async {
        println!("Downloading {} from lang-hub...", lang);
        let manager = crate::indexer::parser::lang_manager::LanguageManager::new(None, None)
            .map_err(|e| anyhow::anyhow!("Failed to init LangManager: {}", e))?;
        
        manager.install_from_github(&lang).await
            .map_err(|e| anyhow::anyhow!("Install failed: {}", e))?;
            
        println!("Successfully installed {}", lang);
        anyhow::Ok(())
    })
}

fn handle_config(action: Option<ConfigAction>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let gofer_dir = cwd.join(".gofer");
    let config_path = gofer_dir.join("config.toml");

    match action {
        Some(ConfigAction::Path) => {
            println!("{}", config_path.display());
        }
        Some(ConfigAction::Init) => {
            std::fs::create_dir_all(&gofer_dir)?;
            if config_path.exists() {
                eprintln!("Config already exists: {}", config_path.display());
                return Ok(());
            }
            std::fs::write(&config_path, DEFAULT_CONFIG)?;
            println!("Created: {}", config_path.display());
        }
        None => {
            // Show effective config
            let config = indexer::watcher::load_config(&gofer_dir);
            println!("# Effective config ({})\n", config_path.display());
            println!("{}", toml::to_string_pretty(&config)?);
        }
    }

    Ok(())
}



const DEFAULT_CONFIG: &str = r#"# gofer configuration
# See: gofer config --help

[server]
port = 10987

[indexer]
# Directories and patterns to exclude from indexing
ignore = [
    "**/node_modules/**",
    "**/target/**",
    "**/.git/**",
    "**/dist/**",
    "**/build/**",
    "**/.next/**",
    "**/.nuxt/**",
    "**/.cache/**",
    "**/coverage/**",
    "**/__pycache__/**",
    "**/.pytest_cache/**",
    "**/vendor/**",
    "**/.venv/**",
    "**/venv/**",
    "**/out/**",
    "**/.idea/**",
    "**/.vscode/**",
    "**/tmp/**",
    "**/temp/**",
]
parallel_workers = 4

[embedding]
batch_size = 32
model = "NomicEmbedTextV15"
pool_size = 4

[reranker]
enabled = true
model_dir = ".gofer/data/models/reranker"

[domains]
rs_paths = []
py_paths = []
frontend_paths = []
ops_paths = []
shared_paths = []
"#;
