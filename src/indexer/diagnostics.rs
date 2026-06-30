use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::storage::SqliteStorage;

// ---------------------------------------------------------------------------
// Rate limiter — prevents excessive diagnostic/verify_patch calls
// ---------------------------------------------------------------------------

/// Minimum interval between diagnostic runs (in seconds).
const DIAGNOSTICS_COOLDOWN_SECS: u64 = 10;
/// Minimum interval between verify_patch calls (in seconds).
const VERIFY_COOLDOWN_SECS: u64 = 3;

static LAST_DIAGNOSTICS: std::sync::LazyLock<Mutex<Option<Instant>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

static LAST_VERIFY: std::sync::LazyLock<Mutex<Option<Instant>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Check if a rate-limited action is allowed; updates the timestamp if allowed.
async fn rate_check(slot: &Mutex<Option<Instant>>, cooldown_secs: u64) -> bool {
    let mut guard = slot.lock().await;
    let now = Instant::now();
    if let Some(last) = *guard {
        if now.duration_since(last).as_secs() < cooldown_secs {
            return false;
        }
    }
    *guard = Some(now);
    true
}

/// Cargo check JSON message format
#[derive(Debug, Deserialize)]
pub struct CargoMessage {
    pub reason: String,
    #[serde(default)]
    pub message: Option<CompilerMessage>,
}

#[derive(Debug, Deserialize)]
pub struct CompilerMessage {
    pub level: String,
    pub message: String,
    pub code: Option<DiagnosticCode>,
    pub spans: Vec<DiagnosticSpan>,
    #[serde(default)]
    pub children: Vec<CompilerMessage>,
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticCode {
    pub code: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DiagnosticSpan {
    pub file_name: String,
    pub line_start: u32,
    pub line_end: u32,
    pub column_start: u32,
    pub column_end: u32,
    pub is_primary: bool,
    #[serde(default)]
    pub suggested_replacement: Option<String>,
}

/// TypeScript diagnostic format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TscDiagnostic {
    file: String,
    line: u32,
    column: u32,
    code: String,
    message: String,
}

#[derive(Debug, Default)]
pub struct CargoCheckOptions {
    pub workspace: Option<bool>,
    pub all_targets: Option<bool>,
    pub package: Option<String>,
    pub manifest_path: Option<String>,
}

/// Status of one diagnostic backend run.
///
/// Distinguishing `Skipped` from `Ran(0,0)` is the whole point: a monorepo
/// without `Cargo.toml` at the root used to silently report `cargo: 0/0`,
/// which looked like "no errors" rather than "didn't run". Now callers can
/// see which backends actually executed.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CheckStatus {
    Ran { errors: usize, warnings: usize },
    Skipped { reason: String },
    Error { message: String },
}

impl CheckStatus {
    pub fn errors(&self) -> usize {
        if let CheckStatus::Ran { errors, .. } = self {
            *errors
        } else {
            0
        }
    }
    pub fn warnings(&self) -> usize {
        if let CheckStatus::Ran { warnings, .. } = self {
            *warnings
        } else {
            0
        }
    }
}

/// Run cargo check and collect diagnostics
pub async fn run_cargo_check(
    root: &Path,
    sqlite: &SqliteStorage,
    options: CargoCheckOptions,
) -> anyhow::Result<CheckStatus> {
    let cargo_toml = root.join("Cargo.toml");
    let manifest_override = options.manifest_path.as_ref().map(|p| root.join(p));
    if !cargo_toml.exists() && !manifest_override.as_ref().map(|p| p.exists()).unwrap_or(true) {
        return Ok(CheckStatus::Skipped {
            reason: format!(
                "no Cargo.toml at {} and manifest_path not found",
                root.display()
            ),
        });
    }
    if !cargo_toml.exists() && manifest_override.is_none() {
        return Ok(CheckStatus::Skipped {
            reason: format!("no Cargo.toml at {}", root.display()),
        });
    }

    tracing::info!("Running cargo check with options: {:?}", options);

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("check").arg("--message-format=json").arg("--quiet");

    if options.workspace.unwrap_or(false) {
        cmd.arg("--workspace");
    }
    if options.all_targets.unwrap_or(false) {
        cmd.arg("--all-targets");
    }
    if let Some(pkg) = options.package {
        cmd.arg("-p").arg(pkg);
    }
    if let Some(manifest) = options.manifest_path {
        cmd.arg("--manifest-path").arg(manifest);
    }

    let output = match cmd.current_dir(root).output().await {
        Ok(o) => o,
        Err(e) => {
            return Ok(CheckStatus::Error {
                message: format!("cargo failed to spawn: {}", e),
            });
        }
    };

    // Clear existing errors
    sqlite.clear_active_errors().await?;

    let mut error_count = 0;
    let mut warning_count = 0;

    // Parse JSON output line by line
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(msg) = serde_json::from_str::<CargoMessage>(line) {
            if msg.reason == "compiler-message" {
                if let Some(compiler_msg) = msg.message {
                    process_cargo_message(
                        &compiler_msg,
                        sqlite,
                        &mut error_count,
                        &mut warning_count,
                    )
                    .await?;
                }
            }
        }
    }

    tracing::info!(
        "Cargo check complete: {} errors, {} warnings",
        error_count,
        warning_count
    );
    Ok(CheckStatus::Ran {
        errors: error_count,
        warnings: warning_count,
    })
}

async fn process_cargo_message(
    msg: &CompilerMessage,
    sqlite: &SqliteStorage,
    error_count: &mut usize,
    warning_count: &mut usize,
) -> anyhow::Result<()> {
    let severity = match msg.level.as_str() {
        "error" => {
            *error_count += 1;
            "error"
        }
        "warning" => {
            *warning_count += 1;
            "warning"
        }
        _ => return Ok(()),
    };

    let code = msg.code.as_ref().map(|c| c.code.as_str());

    // Get suggestion from children
    let suggestion = msg
        .children
        .iter()
        .find(|c| c.level == "help" || c.level == "note")
        .map(|c| c.message.clone());

    // Find primary span
    if let Some(span) = msg.spans.iter().find(|s| s.is_primary) {
        sqlite
            .insert_error(
                &span.file_name,
                span.line_start as i32,
                Some(span.column_start as i32),
                severity,
                code,
                &msg.message,
                suggestion.as_deref(),
            )
            .await?;
    }

    Ok(())
}

/// TypeScript check is disabled in language-agnostic mode.
pub async fn run_tsc_check(_root: &Path, _sqlite: &SqliteStorage) -> anyhow::Result<CheckStatus> {
    Ok(CheckStatus::Skipped {
        reason: "tsc check disabled (language-agnostic mode)".to_string(),
    })
}

/// Run all available diagnostics (rate-limited)
pub async fn run_diagnostics(
    root: &Path,
    sqlite: &SqliteStorage,
    options: CargoCheckOptions,
) -> anyhow::Result<DiagnosticsResult> {
    if !rate_check(&LAST_DIAGNOSTICS, DIAGNOSTICS_COOLDOWN_SECS).await {
        let skipped = CheckStatus::Skipped {
            reason: "rate-limited".to_string(),
        };
        return Ok(DiagnosticsResult {
            total_errors: 0,
            total_warnings: 0,
            cargo: skipped.clone(),
            tsc: skipped,
        });
    }

    sqlite.clear_active_errors().await?;

    let cargo = run_cargo_check(root, sqlite, options)
        .await
        .unwrap_or_else(|e| CheckStatus::Error {
            message: e.to_string(),
        });
    let tsc = run_tsc_check(root, sqlite)
        .await
        .unwrap_or_else(|e| CheckStatus::Error {
            message: e.to_string(),
        });

    Ok(DiagnosticsResult {
        total_errors: cargo.errors() + tsc.errors(),
        total_warnings: cargo.warnings() + tsc.warnings(),
        cargo,
        tsc,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticsResult {
    pub total_errors: usize,
    pub total_warnings: usize,
    pub cargo: CheckStatus,
    pub tsc: CheckStatus,
}

// === verify_patch: Sandboxed Verification ===

/// Глобальный мьютекс для защиты swap & revert от параллельного доступа
static VERIFY_MUTEX: std::sync::LazyLock<Arc<Mutex<()>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(())));

/// Результат верификации патча
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyResult {
    pub status: String, // "success" | "error" | "skipped"
    pub diagnostics: Vec<VerifyDiagnostic>,
    pub summary: String,
}

/// Одна ошибка/предупреждение
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyDiagnostic {
    pub line: u32,
    pub column: Option<u32>,
    pub severity: String, // "error" | "warning"
    pub code: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Определяет тип проверки по расширению файла
fn detect_check_kind(file_path: &Path) -> Option<CheckKind> {
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => Some(CheckKind::Cargo),
        "ts" | "tsx" | "js" | "jsx" | "vue" => Some(CheckKind::Tsc),
        "py" => Some(CheckKind::Ruff),
        _ => None,
    }
}

enum CheckKind {
    Cargo,
    Tsc,
    Ruff,
}

/// Основная точка входа: копирует проект во временную директорию, подменяет файл
/// там и запускает проверку. Оригинальные файлы НИКОГДА не модифицируются.
/// Защищён мьютексом: одновременно может работать только одна верификация.
/// Rate-limited: rejects calls that arrive within VERIFY_COOLDOWN_SECS of the last call.
pub async fn verify_patch(
    root: &Path,
    file_path: &str,
    patch_content: &str,
) -> anyhow::Result<VerifyResult> {
    if !rate_check(&LAST_VERIFY, VERIFY_COOLDOWN_SECS).await {
        return Ok(VerifyResult {
            status: "skipped".into(),
            diagnostics: Vec::new(),
            summary:
                "Rate-limited: слишком частые вызовы verify_patch, повторите через несколько секунд"
                    .into(),
        });
    }

    let abs_path = root.join(file_path);

    if !abs_path.exists() {
        return Ok(VerifyResult {
            status: "error".into(),
            diagnostics: vec![VerifyDiagnostic {
                line: 0,
                column: None,
                severity: "error".into(),
                code: None,
                message: format!("Файл не найден: {}", file_path),
                suggestion: None,
            }],
            summary: "Файл не найден".into(),
        });
    }

    let check_kind = match detect_check_kind(&abs_path) {
        Some(k) => k,
        None => {
            return Ok(VerifyResult {
                status: "skipped".into(),
                diagnostics: Vec::new(),
                summary: format!("Нет подходящего чекера для файла: {}", file_path),
            });
        }
    };

    // Захватываем мьютекс
    let _guard = VERIFY_MUTEX.lock().await;

    // Tmpdir-based подход: создаём временную директорию, копируем файл туда,
    // записываем патч и запускаем проверку. Оригинал не трогаем.
    let tmp_dir = tempfile::tempdir()?;
    let tmp_root = tmp_dir.path();

    // Для Cargo-проектов нужна полная структура — создаём symlink на всё кроме целевого файла
    match check_kind {
        CheckKind::Cargo => {
            // Создаём overlay: symlink корневых элементов + реальная копия целевого файла
            setup_cargo_overlay(root, tmp_root, file_path, patch_content).await?;
            let result = run_cargo_verify(tmp_root, file_path).await;
            // tmp_dir будет удалён автоматически при drop
            result
        }
        CheckKind::Tsc => {
            setup_overlay(root, tmp_root, file_path, patch_content).await?;
            let result = run_tsc_verify(tmp_root, file_path).await;
            result
        }
        CheckKind::Ruff => {
            // Ruff проверяет одиночный файл — просто записываем патч во временный файл
            let tmp_file = tmp_root.join(file_path);
            if let Some(parent) = tmp_file.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&tmp_file, patch_content.as_bytes()).await?;
            let result = run_ruff_verify(&tmp_file).await;
            result
        }
    }
}

/// Создаёт overlay-директорию для Cargo-проекта: symlinks на всё + реальный файл с патчем.
async fn setup_cargo_overlay(
    root: &Path,
    tmp_root: &Path,
    file_path: &str,
    patch_content: &str,
) -> anyhow::Result<()> {
    // Symlink корневые элементы (Cargo.toml, Cargo.lock, src/, etc.)
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Пропускаем target/ (тяжёлый), .git/ и .gofer/
        if name_str == "target" || name_str == ".git" || name_str == ".gofer" {
            continue;
        }
        let src = entry.path();
        let dst = tmp_root.join(&name);
        tokio::fs::symlink(&src, &dst).await?;
    }

    // Symlink на target/ от корня (чтобы cargo check переиспользовал кэш)
    let target_src = root.join("target");
    if target_src.exists() {
        tokio::fs::symlink(&target_src, &tmp_root.join("target")).await?;
    }

    // Теперь "раскрываем" путь к целевому файлу: если file_path = "src/foo/bar.rs",
    // то нужно заменить symlink `src/` на реальную директорию с symlinks внутри,
    // кроме конечного файла.
    replace_symlink_chain(root, tmp_root, file_path, patch_content).await?;

    Ok(())
}

/// Создаёт overlay-директорию для TSC-проекта.
async fn setup_overlay(
    root: &Path,
    tmp_root: &Path,
    file_path: &str,
    patch_content: &str,
) -> anyhow::Result<()> {
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "node_modules" || name_str == ".git" || name_str == ".gofer" {
            continue;
        }
        let src = entry.path();
        let dst = tmp_root.join(&name);
        tokio::fs::symlink(&src, &dst).await?;
    }

    // Symlink на node_modules
    let nm_src = root.join("node_modules");
    if nm_src.exists() {
        tokio::fs::symlink(&nm_src, &tmp_root.join("node_modules")).await?;
    }

    replace_symlink_chain(root, tmp_root, file_path, patch_content).await?;
    Ok(())
}

/// "Раскрывает" symlink-цепочку до целевого файла:
/// Для каждого сегмента пути удаляет symlink и создаёт реальную директорию
/// с symlinks на содержимое оригинальной директории, кроме следующего сегмента.
/// Конечный файл записывается с содержимым патча.
async fn replace_symlink_chain(
    root: &Path,
    tmp_root: &Path,
    file_path: &str,
    patch_content: &str,
) -> anyhow::Result<()> {
    let parts: Vec<&str> = file_path.split('/').collect();
    if parts.is_empty() {
        return Ok(());
    }

    let mut real_prefix = root.to_path_buf();
    let mut tmp_prefix = tmp_root.to_path_buf();

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Конечный файл — записываем патч
            let dst = tmp_prefix.join(part);
            tokio::fs::write(&dst, patch_content.as_bytes()).await?;
        } else {
            // Промежуточная директория — раскрываем symlink
            let tmp_dir = tmp_prefix.join(part);
            let real_dir = real_prefix.join(part);

            // Удаляем symlink если он есть
            if tokio::fs::symlink_metadata(&tmp_dir).await.is_ok() {
                tokio::fs::remove_file(&tmp_dir).await?;
            }

            // Создаём реальную директорию
            tokio::fs::create_dir_all(&tmp_dir).await?;

            // Заполняем symlinks на содержимое оригинальной директории
            if real_dir.is_dir() {
                let mut entries = tokio::fs::read_dir(&real_dir).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let name = entry.file_name();
                    let next_part = parts.get(i + 1).map(|s| std::ffi::OsStr::new(*s));
                    // Пропускаем следующий сегмент — его мы обработаем на следующей итерации
                    if Some(name.as_os_str()) == next_part {
                        continue;
                    }
                    let src = entry.path();
                    let dst = tmp_dir.join(&name);
                    tokio::fs::symlink(&src, &dst).await?;
                }
            }

            tmp_prefix = tmp_dir;
            real_prefix = real_dir;
        }
    }

    Ok(())
}

/// Cargo check — парсим JSON, фильтруем только ошибки для нашего файла
async fn run_cargo_verify(root: &Path, file_path: &str) -> anyhow::Result<VerifyResult> {
    let output = tokio::process::Command::new("cargo")
        .arg("check")
        .arg("--message-format=json")
        .arg("--quiet")
        .current_dir(root)
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut diagnostics = Vec::new();

    for line in stdout.lines() {
        let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
            continue;
        };

        if msg.reason != "compiler-message" {
            continue;
        }

        let Some(compiler_msg) = msg.message else {
            continue;
        };

        collect_cargo_diagnostics(&compiler_msg, file_path, &mut diagnostics);
    }

    let error_count = diagnostics.iter().filter(|d| d.severity == "error").count();
    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count();

    let status = if error_count > 0 { "error" } else { "success" };
    let summary = format!("{} errors, {} warnings", error_count, warning_count);

    Ok(VerifyResult {
        status: status.into(),
        diagnostics,
        summary,
    })
}

fn collect_cargo_diagnostics(
    msg: &CompilerMessage,
    file_path: &str,
    diagnostics: &mut Vec<VerifyDiagnostic>,
) {
    let severity = match msg.level.as_str() {
        "error" => "error",
        "warning" => "warning",
        _ => return,
    };

    let suggestion = msg
        .children
        .iter()
        .find(|c| c.level == "help" || c.level == "note")
        .map(|c| c.message.clone());

    // Фильтруем по файлу: берём spans, где file_name заканчивается на наш файл
    for span in &msg.spans {
        if !span.is_primary {
            continue;
        }
        // Сравниваем: span.file_name может быть "src/foo.rs", file_path — "src/foo.rs"
        if span.file_name.ends_with(file_path) || file_path.ends_with(&span.file_name) {
            diagnostics.push(VerifyDiagnostic {
                line: span.line_start,
                column: Some(span.column_start),
                severity: severity.into(),
                code: msg.code.as_ref().map(|c| c.code.clone()),
                message: msg.message.clone(),
                suggestion: suggestion.clone(),
            });
        }
    }

    // Если нет spans с нашим файлом, но есть spans вообще — это может быть
    // ошибка в другом файле, вызванная нашим изменением. Добавляем тоже.
    if (diagnostics.is_empty() || msg.spans.is_empty())
        && severity == "error"
        && msg.spans.is_empty()
    {
        diagnostics.push(VerifyDiagnostic {
            line: 0,
            column: None,
            severity: severity.into(),
            code: msg.code.as_ref().map(|c| c.code.clone()),
            message: msg.message.clone(),
            suggestion,
        });
    }
}

/// TypeScript/Vue check — tsc --noEmit, парсим текстовый вывод
async fn run_tsc_verify(root: &Path, file_path: &str) -> anyhow::Result<VerifyResult> {
    // Определяем команду
    let (cmd, args) = if root.join("node_modules/.bin/vue-tsc").exists() {
        (
            "./node_modules/.bin/vue-tsc",
            vec!["--noEmit", "--pretty", "false"],
        )
    } else if root.join("node_modules/.bin/tsc").exists() {
        (
            "./node_modules/.bin/tsc",
            vec!["--noEmit", "--pretty", "false"],
        )
    } else {
        ("npx", vec!["tsc", "--noEmit", "--pretty", "false"])
    };

    let output = tokio::process::Command::new(cmd)
        .args(&args)
        .current_dir(root)
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(_) => {
            return Ok(VerifyResult {
                status: "skipped".into(),
                diagnostics: Vec::new(),
                summary: "TypeScript compiler not available".into(),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    let re = regex::Regex::new(r"^(.+?)\((\d+),(\d+)\):\s*(error|warning)\s+(TS\d+):\s*(.+)$")?;

    let mut diagnostics = Vec::new();

    for line in combined.lines() {
        let Some(caps) = re.captures(line) else {
            continue;
        };

        let file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let line_num: u32 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let col: u32 = caps
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let severity = caps.get(4).map(|m| m.as_str()).unwrap_or("error");
        let code = caps.get(5).map(|m| m.as_str().to_string());
        let message = caps.get(6).map(|m| m.as_str()).unwrap_or("");

        // Фильтруем по файлу
        if file.ends_with(file_path) || file_path.ends_with(file) {
            diagnostics.push(VerifyDiagnostic {
                line: line_num,
                column: Some(col),
                severity: severity.into(),
                code,
                message: message.into(),
                suggestion: None,
            });
        }
    }

    let error_count = diagnostics.iter().filter(|d| d.severity == "error").count();
    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count();

    let status = if error_count > 0 { "error" } else { "success" };
    let summary = format!("{} errors, {} warnings", error_count, warning_count);

    Ok(VerifyResult {
        status: status.into(),
        diagnostics,
        summary,
    })
}

/// Python: ruff check через stdin
async fn run_ruff_verify(file_path: &Path) -> anyhow::Result<VerifyResult> {
    let content = tokio::fs::read_to_string(file_path).await?;
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("stdin.py");

    let output = tokio::process::Command::new("ruff")
        .args([
            "check",
            "--output-format=json",
            "--stdin-filename",
            file_name,
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match output {
        Ok(c) => c,
        Err(_) => {
            // ruff не установлен — попробуем python -m py_compile
            return run_python_syntax_check(file_path).await;
        }
    };

    // Передаём содержимое через stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(content.as_bytes()).await;
        drop(stdin);
    }

    let output = child.wait_with_output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut diagnostics = Vec::new();

    // Парсим JSON-массив ruff
    if let Ok(ruff_results) = serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
        for entry in &ruff_results {
            let message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let code = entry
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let row = entry
                .get("location")
                .and_then(|l| l.get("row"))
                .and_then(|r| r.as_u64())
                .unwrap_or(0) as u32;
            let col = entry
                .get("location")
                .and_then(|l| l.get("column"))
                .and_then(|c| c.as_u64())
                .map(|c| c as u32);

            diagnostics.push(VerifyDiagnostic {
                line: row,
                column: col,
                severity: "error".into(),
                code,
                message: message.into(),
                suggestion: entry
                    .get("fix")
                    .and_then(|f| f.get("message"))
                    .and_then(|m| m.as_str())
                    .map(|s| s.into()),
            });
        }
    }

    let error_count = diagnostics.len();
    let status = if error_count > 0 { "error" } else { "success" };
    let summary = format!("{} issues", error_count);

    Ok(VerifyResult {
        status: status.into(),
        diagnostics,
        summary,
    })
}

/// Fallback: проверка синтаксиса Python через py_compile
async fn run_python_syntax_check(file_path: &Path) -> anyhow::Result<VerifyResult> {
    let path_str = file_path.to_string_lossy();

    let output = tokio::process::Command::new("python3")
        .args(["-m", "py_compile", &path_str])
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(_) => {
            return Ok(VerifyResult {
                status: "skipped".into(),
                diagnostics: Vec::new(),
                summary: "No Python checker available".into(),
            });
        }
    };

    if output.status.success() {
        return Ok(VerifyResult {
            status: "success".into(),
            diagnostics: Vec::new(),
            summary: "Syntax OK".into(),
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Парсим "SyntaxError" из py_compile вывода
    let re = regex::Regex::new(r"line (\d+)")?;
    let line = re
        .captures(&stderr)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .unwrap_or(0);

    Ok(VerifyResult {
        status: "error".into(),
        diagnostics: vec![VerifyDiagnostic {
            line,
            column: None,
            severity: "error".into(),
            code: None,
            message: stderr.trim().to_string(),
            suggestion: None,
        }],
        summary: "Syntax error".into(),
    })
}
