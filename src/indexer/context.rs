//! Context bundle utilities — creates dependency bundles and skeletonized views.
//!
//! Moved from api/context.rs; no HTTP/axum dependencies.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::indexer::parser::{self, CodeParser, SupportedLanguage};
use crate::models::{ContextBundle, DependencyFile};

pub async fn create_bundle(main_path: &Path, max_depth: u32) -> ContextBundle {
    let main_content = tokio::fs::read_to_string(main_path)
        .await
        .unwrap_or_default();
    let mut dependencies = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(main_path.canonicalize().unwrap_or(main_path.to_path_buf()));

    let ext = main_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if let Some(language) = SupportedLanguage::from_extension(ext) {
        let mut parser = CodeParser::new();
        let imports = parser.parse_imports(&main_content, language.clone());
        let base_dir = main_path.parent().unwrap_or(Path::new("."));

        for import in imports {
            if import.is_relative {
                if let Some(resolved) = resolve_import(&import.path, base_dir, language.clone()).await {
                    collect_dependency(
                        &resolved,
                        &import.items.join(", "),
                        &mut dependencies,
                        &mut visited,
                        max_depth,
                        1,
                    )
                    .await;
                }
            }
        }
    }

    let markdown = generate_markdown(main_path, &main_content, &dependencies);

    let main_lines = main_content.lines().count();
    let dep_lines: usize = dependencies.iter().map(|d| d.content.lines().count()).sum();
    let total_lines = main_lines + dep_lines;
    let total_chars =
        main_content.len() + dependencies.iter().map(|d| d.content.len()).sum::<usize>();
    let total_tokens_estimate = total_chars / 4;

    ContextBundle {
        main_file: main_path.to_string_lossy().to_string(),
        main_content,
        dependencies,
        markdown,
        total_lines,
        total_tokens_estimate,
    }
}

fn resolve_import<'a>(
    import_path: &'a str,
    base_dir: &'a Path,
    language: SupportedLanguage,
) -> core::pin::Pin<Box<dyn core::future::Future<Output = Option<PathBuf>> + Send + 'a>> {
    Box::pin(async move {
        let normalized = import_path
            .trim_start_matches("./")
            .trim_start_matches("@/");

        let extensions: &[&str] = match language.name() {
            "rust" => &["rs"],
            "typescript" => &["ts", "tsx", "js", "jsx"],
            "javascript" => &["js", "jsx", "ts", "tsx"],
            "vue" => &["vue", "ts", "js"],
            "python" => &["py"],
            "go" => &["go"],
         _ => &[], };

        // 1. Прямой путь: base_dir/normalized.ext
        for ext in extensions {
            let candidate = base_dir.join(normalized).with_added_extension(ext);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // 2. Index файл: base_dir/normalized/index.ext
        for ext in extensions {
            let candidate = base_dir
                .join(normalized)
                .join("index")
                .with_added_extension(ext);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // 3. Vue/TS `@/` alias → ищем tsconfig.json paths или src/
        if import_path.starts_with("@/") {
            let rel_path = import_path.trim_start_matches("@/");

            // Попробовать найти tsconfig.json и прочитать paths
            if let Some(resolved) = resolve_tsconfig_paths(import_path, base_dir, extensions).await
            {
                return Some(resolved);
            }

            // Fallback: @/ → src/
            let src_path = base_dir
                .ancestors()
                .find(|p| p.join("src").exists())
                .map(|p| p.join("src"));

            if let Some(src) = src_path {
                for ext in extensions {
                    let candidate = src.join(rel_path).with_added_extension(ext);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
                for ext in extensions {
                    let candidate = src.join(rel_path).join("index").with_added_extension(ext);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }

        // 4. Python relative imports (from ..foo import bar → base_dir/../../foo.py)
        if language.name() == "python" && import_path.starts_with('.') {
            let dots = import_path.chars().take_while(|c| *c == '.').count();
            let module = &import_path[dots..];
            let mut target_dir = base_dir.to_path_buf();
            for _ in 1..dots {
                target_dir = target_dir.parent().unwrap_or(base_dir).to_path_buf();
            }
            let module_path = module.replace('.', "/");
            if !module_path.is_empty() {
                // Файл
                let candidate = target_dir.join(&module_path).with_added_extension("py");
                if candidate.exists() {
                    return Some(candidate);
                }
                // Пакет
                let candidate = target_dir.join(&module_path).join("__init__.py");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }

        // 5. Rust mod tree: base_dir/name.rs или base_dir/name/mod.rs
        if language.name() == "rust" && !normalized.contains('/') {
            let candidate = base_dir.join(normalized).with_added_extension("rs");
            if candidate.exists() {
                return Some(candidate);
            }
            let candidate = base_dir.join(normalized).join("mod.rs");
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    })
}

/// Попытка разрешить import через tsconfig.json compilerOptions.paths
async fn resolve_tsconfig_paths(
    import_path: &str,
    base_dir: &Path,
    extensions: &[&str],
) -> Option<PathBuf> {
    // Найти tsconfig.json поднимаясь по директориям
    let tsconfig_path = base_dir
        .ancestors()
        .map(|p| p.join("tsconfig.json"))
        .find(|p| p.exists())?;

    let tsconfig_content = tokio::fs::read_to_string(&tsconfig_path).await.ok()?;
    let tsconfig: serde_json::Value = serde_json::from_str(&tsconfig_content).ok()?;

    let paths = tsconfig
        .get("compilerOptions")
        .and_then(|co| co.get("paths"))
        .and_then(|p| p.as_object())?;

    let tsconfig_dir = tsconfig_path.parent()?;
    let base_url = tsconfig
        .get("compilerOptions")
        .and_then(|co| co.get("baseUrl"))
        .and_then(|b| b.as_str())
        .unwrap_or(".");
    let base_url_dir = tsconfig_dir.join(base_url);

    for (pattern, targets) in paths {
        // Паттерн вида "@/*" → targets ["src/*"]
        let pattern_prefix = pattern.trim_end_matches('*');
        if let Some(remainder) = import_path.strip_prefix(pattern_prefix) {
            let targets_arr = targets.as_array()?;
            for target in targets_arr {
                let target_str = target.as_str()?;
                let target_prefix = target_str.trim_end_matches('*');
                let resolved_base = base_url_dir.join(target_prefix);
                // Попробовать с каждым расширением
                for ext in extensions {
                    let candidate = resolved_base.join(remainder).with_added_extension(ext);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
                // Index файл
                for ext in extensions {
                    let candidate = resolved_base
                        .join(remainder)
                        .join("index")
                        .with_added_extension(ext);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    None
}

fn collect_dependency<'a>(
    path: &'a Path,
    reason: &'a str,
    deps: &'a mut Vec<DependencyFile>,
    visited: &'a mut HashSet<PathBuf>,
    max_depth: u32,
    current_depth: u32,
) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let canonical = path.canonicalize().unwrap_or(path.to_path_buf());
        if visited.contains(&canonical) || current_depth > max_depth {
            return;
        }
        visited.insert(canonical.clone());

        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => {
                visited.remove(&canonical);
                return;
            }
        };

        deps.push(DependencyFile {
            path: path.to_string_lossy().to_string(),
            content: content.clone(),
            reason: format!("imports: {}", reason),
            depth: current_depth,
        });

        if current_depth < max_depth {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if let Some(language) = SupportedLanguage::from_extension(ext) {
                let mut parser = CodeParser::new();
                let imports = parser.parse_imports(&content, language.clone());
                let base_dir = path.parent().unwrap_or(Path::new("."));

                for import in imports {
                    if import.is_relative {
                        if let Some(resolved) =
                            resolve_import(&import.path, base_dir, language.clone()).await
                        {
                            collect_dependency(
                                &resolved,
                                &import.items.join(", "),
                                deps,
                                visited,
                                max_depth,
                                current_depth + 1,
                            )
                            .await;
                        }
                    }
                }
            }
        }
    })
}

fn generate_markdown(main_path: &Path, main_content: &str, deps: &[DependencyFile]) -> String {
    let mut md = String::new();

    let ext = main_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");
    md.push_str(&format!("# Main File: `{}`\n\n", main_path.display()));
    md.push_str(&format!("```{}\n{}\n```\n\n", ext, main_content));

    if !deps.is_empty() {
        md.push_str("---\n\n# Dependencies\n\n");
        for dep in deps {
            let dep_ext = Path::new(&dep.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("txt");
            md.push_str(&format!("## `{}` ({})\n\n", dep.path, dep.reason));
            md.push_str(&format!("```{}\n{}\n```\n\n", dep_ext, dep.content));
        }
    }

    md
}

/// Skeletonize a bundle (main_content + dependencies).
pub fn skeletonize_bundle(bundle: &mut ContextBundle) {
    let ext = Path::new(&bundle.main_file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if let Some(lang) = SupportedLanguage::from_extension(ext) {
        if let Ok(skeleton) = parser::generate_skeleton(&bundle.main_content, lang) {
            bundle.main_content = skeleton;
        }
    }

    for dep in &mut bundle.dependencies {
        let dep_ext = Path::new(&dep.path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if let Some(lang) = SupportedLanguage::from_extension(dep_ext) {
            if let Ok(skeleton) = parser::generate_skeleton(&dep.content, lang) {
                dep.content = skeleton;
            }
        }
    }

    let main_path = Path::new(&bundle.main_file);
    bundle.markdown = generate_markdown(main_path, &bundle.main_content, &bundle.dependencies);

    let main_lines = bundle.main_content.lines().count();
    let dep_lines: usize = bundle
        .dependencies
        .iter()
        .map(|d| d.content.lines().count())
        .sum();
    bundle.total_lines = main_lines + dep_lines;
    let total_chars = bundle.main_content.len()
        + bundle
            .dependencies
            .iter()
            .map(|d| d.content.len())
            .sum::<usize>();
    bundle.total_tokens_estimate = total_chars / 4;
}

/// Skeletonize only dependencies, keeping main_content intact.
pub fn skeletonize_deps_only(bundle: &mut ContextBundle) {
    for dep in &mut bundle.dependencies {
        let dep_ext = Path::new(&dep.path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if let Some(lang) = SupportedLanguage::from_extension(dep_ext) {
            if let Ok(skeleton) = parser::generate_skeleton(&dep.content, lang) {
                dep.content = skeleton;
            }
        }
    }

    let main_path = Path::new(&bundle.main_file);
    bundle.markdown = generate_markdown(main_path, &bundle.main_content, &bundle.dependencies);

    let main_lines = bundle.main_content.lines().count();
    let dep_lines: usize = bundle
        .dependencies
        .iter()
        .map(|d| d.content.lines().count())
        .sum();
    bundle.total_lines = main_lines + dep_lines;
    let total_chars = bundle.main_content.len()
        + bundle
            .dependencies
            .iter()
            .map(|d| d.content.len())
            .sum::<usize>();
    bundle.total_tokens_estimate = total_chars / 4;
}

/// Skeletonize a single file by content and extension.
pub fn skeletonize_content(content: &str, extension: &str) -> String {
    let Some(lang) = SupportedLanguage::from_extension(extension) else {
        return content.to_string();
    };
    parser::generate_skeleton(content, lang).unwrap_or_else(|_| content.to_string())
}
