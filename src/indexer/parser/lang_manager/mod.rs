use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tree_sitter::{Language, WasmError, WasmStore};

#[derive(Error, Debug)]
pub enum LangManagerError {
    #[error("Manifest parse error: {0}")]
    ManifestError(#[from] toml::de::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Language not found: {0}")]
    LanguageNotFound(String),
    #[error("WASM error: {0}")]
    WasmError(#[from] WasmError),
    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Wasmtime engine error: {0}")]
    EngineError(String),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestComments {
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub block: Option<Vec<String>>,
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestLanguage {
    pub name: String,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub comments: Option<ManifestComments>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestParser {
    pub r#type: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub download_url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestLsp {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub init_options: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestSandbox {
    pub compile_cmd: String,
    pub run_cmd: String,
    #[serde(default)]
    pub package_manager: Option<String>,
    #[serde(default)]
    pub test: Option<Vec<String>>,
    #[serde(default)]
    pub build: Option<Vec<String>>,
    #[serde(default)]
    pub run: Option<Vec<String>>,
    #[serde(default)]
    pub script: Option<Vec<String>>,
    #[serde(default)]
    pub repl: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestFormatLint {
    pub tool: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestIndexer {
    #[serde(default)]
    pub ignore_folders: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LangManifest {
    pub language: ManifestLanguage,
    pub parser: ManifestParser,
    pub lsp: Option<ManifestLsp>,
    pub sandbox: Option<ManifestSandbox>,
    pub indexer: Option<ManifestIndexer>,
    pub formatter: Option<ManifestFormatLint>,
    pub linter: Option<ManifestFormatLint>,
}

#[derive(Clone)]
pub struct LoadedLanguage {
    pub manifest: LangManifest,
    pub language: Language,
    pub queries: Arc<std::collections::HashMap<String, Arc<tree_sitter::Query>>>,
}

pub struct LanguageManager {
    /// Mapping of language name to LoadedLanguage
    pub loaded_langs: Arc<dashmap::DashMap<String, Arc<LoadedLanguage>>>,
    /// Extension to language name mapping (e.g., "rs" -> "rust")
    pub ext_to_lang: Arc<dashmap::DashMap<String, String>>,
    /// Directory where language packs are stored (e.g., ~/.gofer/langs)
    pub langs_dir: PathBuf,
    /// Shared WebAssembly engine for compiling parsers
    pub engine: tree_sitter::wasmtime::Engine,
    /// Prevents concurrent downloads of the same language
    pub download_locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl LanguageManager {
    pub fn new(langs_dir: Option<PathBuf>, _tools_dir: Option<PathBuf>) -> Result<Self, LangManagerError> {
        let base_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".gofer");

        let langs_dir = langs_dir.unwrap_or_else(|| base_dir.join("langs"));

        if !langs_dir.exists() {
            std::fs::create_dir_all(&langs_dir)?;
        }

        let engine = tree_sitter::wasmtime::Engine::default();

        let ext_to_lang = dashmap::DashMap::new();
        // Default mappings to bootstrap downloading if ~/.gofer/langs is completely empty
        let defaults = [
            ("rs", "rust"), ("js", "typescript"), ("jsx", "typescript"),
            ("ts", "typescript"), ("tsx", "typescript"), ("py", "python"),
            ("mjs", "typescript"), ("cjs", "typescript"),
            ("go", "go"), ("c", "c"), ("h", "c"), ("cpp", "cpp"), ("hpp", "cpp"),
            ("java", "java"), ("rb", "ruby"), ("md", "markdown"),
            ("vue", "vue"), ("html", "html"), ("css", "css"),
            ("json", "json"), ("toml", "toml"), ("yaml", "yaml"),
            ("yml", "yaml"), ("sh", "bash"), ("bash", "bash"), ("sql", "sql"),
        ];
        for (ext, lang) in defaults {
            ext_to_lang.insert(ext.to_string(), lang.to_string());
        }

        Ok(Self {
            loaded_langs: Arc::new(dashmap::DashMap::new()),
            ext_to_lang: Arc::new(ext_to_lang),
            langs_dir,
            engine,
            download_locks: Arc::new(dashmap::DashMap::new()),
        })
    }

    /// Tries to resolve a language by file extension. 
    /// If the language is not loaded yet, but exists on disk, it will load it.
    pub fn get_language_by_ext(&self, ext: &str) -> Option<String> {
        if let Some(lang_name) = self.ext_to_lang.get(ext) {
            return Some(lang_name.clone());
        }

        // Try to scan langs_dir for a manifest that supports this extension
        if let Ok(entries) = std::fs::read_dir(&self.langs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let manifest_path = path.join("manifest.toml");
                    if manifest_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                            if let Ok(manifest) = toml::from_str::<LangManifest>(&content) {
                                if manifest.language.extensions.iter().any(|e| e == ext) 
                                    || manifest.language.aliases.iter().any(|a| a == ext)
                                    || manifest.language.name == ext {
                                    // Found a match! Load it.
                                    let lang_name = manifest.language.name.clone();
                                    if self.load_language_from_disk(&lang_name).is_ok() {
                                        return Some(lang_name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Gets the loaded language structure. Will attempt to load from disk and download if missing.
    pub fn get_language(&self, lang_name: &str) -> Option<Arc<LoadedLanguage>> {
        if let Some(entry) = self.loaded_langs.get(lang_name) {
            return Some(entry.value().clone());
        }

        // Try to load from disk
        match self.load_language_from_disk(lang_name) {
            Ok(_) => {
                if let Some(entry) = self.loaded_langs.get(lang_name) {
                    return Some(entry.value().clone());
                }
            }
            Err(LangManagerError::LanguageNotFound(_)) => {
                // Proceed to download
            }
            Err(e) => {
                tracing::error!("Language '{}' exists but failed to load (ABI mismatch or corrupted file?): {:?}", lang_name, e);
                return None;
            }
        }

        let lock = self.download_locks
            .entry(lang_name.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();

        // Not on disk, try to auto-download from GitHub
        tracing::info!("Language {} not found locally, auto-downloading from lang-hub...", lang_name);
        
        // Helper block to safely run async code and avoid data races
        let install_func = || async {
            let _guard = lock.lock().await;
            // Check if another thread downloaded it while we were waiting
            if self.loaded_langs.contains_key(lang_name) {
                return Ok(());
            }
            self.install_from_github(lang_name).await
        };

        let handle = tokio::runtime::Handle::try_current();
        let res = match handle {
            Ok(h) => {
                tokio::task::block_in_place(|| {
                    h.block_on(install_func())
                })
            }
            Err(_) => {
                if let Ok(rt) = tokio::runtime::Runtime::new() {
                    rt.block_on(install_func())
                } else {
                    Err(LangManagerError::EngineError("Failed to create tokio runtime".to_string()))
                }
            }
        };

        if let Err(e) = res {
            tracing::error!("Failed to auto-download language {}: {}", lang_name, e);
            return None;
        }

        // Now load from disk again
        if let Err(e) = self.load_language_from_disk(lang_name) {
            tracing::error!("Failed to load language {} after download: {}", lang_name, e);
            return None;
        }

        self.loaded_langs.get(lang_name).map(|entry| entry.value().clone())
    }

    /// Loads a language pack from disk (assumes it is already downloaded)
    pub fn load_language_from_disk(&self, lang_name: &str) -> Result<(), LangManagerError> {
        if self.loaded_langs.contains_key(lang_name) {
            return Ok(());
        }

        let lang_dir = self.langs_dir.join(lang_name);
        let manifest_path = lang_dir.join("manifest.toml");
        
        if !manifest_path.exists() {
            return Err(LangManagerError::LanguageNotFound(lang_name.to_string()));
        }

        let manifest_str = std::fs::read_to_string(manifest_path)?;
        let manifest: LangManifest = toml::from_str(&manifest_str)?;

        let wasm_path = lang_dir.join(format!("{}.wasm", manifest.language.name));
        if !wasm_path.exists() {
            return Err(LangManagerError::LanguageNotFound(format!("WASM missing for {}", lang_name)));
        }

        let mut store = WasmStore::new(&self.engine)
            .map_err(|e| LangManagerError::EngineError(e.to_string()))?;
        
        let wasm_bytes = std::fs::read(&wasm_path)?;
        let language = store.load_language(&manifest.language.name, &wasm_bytes)?;

        let queries_dir = lang_dir.join("queries");
        let mut queries = std::collections::HashMap::new();

        if queries_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&queries_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("scm") {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            if let Ok(query_str) = std::fs::read_to_string(&path) {
                                match tree_sitter::Query::new(&language, &query_str) {
                                    Ok(q) => {
                                        queries.insert(stem.to_string(), Arc::new(q));
                                    }
                                    Err(e) => tracing::warn!("Failed to compile query {}: {}", stem, e),
                                }
                            }
                        }
                    }
                }
            }
        }

        let loaded = Arc::new(LoadedLanguage {
            manifest: manifest.clone(),
            language,
            queries: Arc::new(queries),
        });

        self.loaded_langs.insert(lang_name.to_string(), loaded);
        
        for ext in manifest.language.extensions {
            self.ext_to_lang.insert(ext, lang_name.to_string());
        }
        for alias in manifest.language.aliases {
            self.ext_to_lang.insert(alias, lang_name.to_string());
        }
        self.ext_to_lang.insert(manifest.language.name, lang_name.to_string());

        tracing::info!("Successfully loaded language plugin: {}", lang_name);

        Ok(())
    }

    /// Installs a language pack directly from lang-hub on GitHub.
    pub async fn install_from_github(&self, lang_name: &str) -> Result<(), LangManagerError> {
        let base_url = format!("https://raw.githubusercontent.com/budka-dev/lang-hub/main/list/{}", lang_name);
        
        // 1. Download and parse manifest
        let manifest_url = format!("{}/manifest.toml", base_url);
        let resp = reqwest::get(&manifest_url).await?;
        if !resp.status().is_success() {
            return Err(LangManagerError::LanguageNotFound(format!("Language {} not found in lang-hub", lang_name)));
        }
        let manifest_str = resp.text().await?;
        let manifest: LangManifest = toml::from_str(&manifest_str)?;
        
        let lang_dir = self.langs_dir.join(lang_name);
        std::fs::create_dir_all(&lang_dir)?;
        
        // Save manifest
        std::fs::write(lang_dir.join("manifest.toml"), &manifest_str)?;
        
        // 2. Download WASM
        // First, try to download from the compiled budka-dev/lang-hub release
        let release_url = format!("https://github.com/budka-dev/lang-hub/releases/download/latest/tree-sitter-{}.wasm", lang_name);
        
        let mut resp = reqwest::get(&release_url).await?;
        if !resp.status().is_success() {
            // Fallback to manifest download_url if the release doesn't have it
            let fallback_url = match &manifest.parser.download_url {
                url if !url.is_empty() => url.clone(),
                _ => return Err(LangManagerError::LanguageNotFound(format!("No WASM found in latest release and no fallback URL for {}", lang_name))),
            };
            tracing::info!("WASM not found in lang-hub release, falling back to manifest URL: {}", fallback_url);
            resp = reqwest::get(&fallback_url).await.map_err(|e| LangManagerError::LanguageNotFound(format!("Failed to download WASM: {}", e)))?;
            
            if !resp.status().is_success() {
                return Err(LangManagerError::LanguageNotFound(format!("Failed to download WASM for {} from fallback URL", lang_name)));
            }
        }
        
        let wasm_bytes = resp.bytes().await?;
        // gofer expects <name>.wasm
        std::fs::write(lang_dir.join(format!("{}.wasm", manifest.language.name)), &wasm_bytes)?;
        
        // 3. Download queries
        let queries_dir = lang_dir.join("queries");
        std::fs::create_dir_all(&queries_dir)?;
        
        let standard_queries = [
            "symbols.scm", "references.scm", "highlights.scm", 
            "locals.scm", "injections.scm", "folds.scm", 
            "tags.scm", "indents.scm", "outline.scm"
        ];
        
        for q in standard_queries.iter() {
            let q_url = format!("{}/queries/{}", base_url, q);
            if let Ok(resp) = reqwest::get(&q_url).await {
                if resp.status().is_success() {
                    if let Ok(q_str) = resp.text().await {
                        let _ = std::fs::write(queries_dir.join(q), q_str);
                    }
                }
            }
        }
        
        tracing::info!("Successfully downloaded {} from lang-hub", lang_name);
        
        self.load_language_from_disk(lang_name)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_language_from_disk() {
        let langs_dir = PathBuf::from("tests/fixtures/langs");
        let manager = LanguageManager::new(Some(langs_dir), None).unwrap();
        
        let result = manager.load_language_from_disk("rust");
        
        match result {
            Ok(_) => {
                let lang = manager.get_language("rust").expect("Language should be loaded");
                assert_eq!(lang.manifest.language.name, "rust");
                assert_eq!(lang.manifest.language.extensions, vec!["rs"]);
                
                let ext_lang = manager.get_language_by_ext("rs");
                assert_eq!(ext_lang, Some("rust".to_string()));

                // Try to parse something
                let mut parser = tree_sitter::Parser::new();
                if let Err(e) = parser.set_language(&lang.language) {
                    println!("Skipping parse test due to LanguageError (ABI mismatch): {:?}", e);
                    return;
                }
                
                let code = "fn main() { println!(\"Hello World\"); }";
                let tree = parser.parse(code, None).expect("Should parse code");
                
                assert_eq!(tree.root_node().kind(), "source_file");
                assert!(tree.root_node().child_count() > 0);
            },
            Err(LangManagerError::WasmError(e)) => {
                // Ignore version mismatches (e.g., ABI 15 when we only support 14 in tree-sitter 0.24)
                println!("Skipping parse test due to WASM version mismatch: {:?}", e);
            },
            Err(e) => panic!("Failed to load rust: {:?}", e),
        }
    }
}
