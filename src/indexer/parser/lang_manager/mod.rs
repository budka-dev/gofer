use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
pub struct ManifestLanguage {
    pub name: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestParser {
    pub r#type: String,
    pub download_url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestLsp {
    pub name: String,
    pub download_url: String,
    pub command: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManifestSandbox {
    pub compile_cmd: String,
    pub run_cmd: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LangManifest {
    pub language: ManifestLanguage,
    pub parser: ManifestParser,
    pub lsp: Option<ManifestLsp>,
    pub sandbox: Option<ManifestSandbox>,
}

pub struct LoadedLanguage {
    pub manifest: LangManifest,
    pub language: Language,
    pub symbols_query: String,
    pub references_query: String,
}

pub struct LanguageManager {
    /// Mapping of language name to LoadedLanguage
    loaded_langs: Arc<dashmap::DashMap<String, LoadedLanguage>>,
    /// Extension to language name mapping (e.g., "rs" -> "rust")
    ext_to_lang: Arc<dashmap::DashMap<String, String>>,
    /// Directory where language packs are stored (e.g., ~/.gofer/langs)
    langs_dir: PathBuf,
    /// Shared WebAssembly engine for compiling parsers
    engine: tree_sitter::wasmtime::Engine,
}

impl LanguageManager {
    pub fn new(langs_dir: Option<PathBuf>) -> Result<Self, LangManagerError> {
        let langs_dir = langs_dir.unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".gofer")
                .join("langs")
        });

        if !langs_dir.exists() {
            std::fs::create_dir_all(&langs_dir)?;
        }

        let engine = tree_sitter::wasmtime::Engine::default();

        Ok(Self {
            loaded_langs: Arc::new(dashmap::DashMap::new()),
            ext_to_lang: Arc::new(dashmap::DashMap::new()),
            langs_dir,
            engine,
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
                                if manifest.language.extensions.iter().any(|e| e == ext) {
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

    /// Gets the loaded language structure. 
    pub fn get_language(&self, lang_name: &str) -> Option<LoadedLanguage> {
        // Since we cannot return a reference to dashmap easily without holding a lock guard,
        // we might need to clone, but `Language` is just a pointer inside, so we can clone it.
        // But for queries, we'd need to clone strings.
        // Let's implement a pattern where we return an owned copy or Arc.
        // For simplicity now, let's just clone.
        if let Some(entry) = self.loaded_langs.get(lang_name) {
            Some(LoadedLanguage {
                manifest: entry.manifest.clone(),
                language: entry.language.clone(),
                symbols_query: entry.symbols_query.clone(),
                references_query: entry.references_query.clone(),
            })
        } else {
            None
        }
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
        let language = store.load_language(&wasm_path.to_string_lossy(), &wasm_bytes)?;

        let queries_dir = lang_dir.join("queries");
        let symbols_query = std::fs::read_to_string(queries_dir.join("symbols.scm")).unwrap_or_default();
        let references_query = std::fs::read_to_string(queries_dir.join("references.scm")).unwrap_or_default();

        let loaded = LoadedLanguage {
            manifest: manifest.clone(),
            language,
            symbols_query,
            references_query,
        };

        self.loaded_langs.insert(lang_name.to_string(), loaded);
        
        for ext in manifest.language.extensions {
            self.ext_to_lang.insert(ext, lang_name.to_string());
        }

        tracing::info!("Successfully loaded language plugin: {}", lang_name);

        Ok(())
    }

    /// Installs a language pack from a git repo or direct URL.
    /// In the future, this should pull the whole pack (manifest + queries + wasm).
    /// For now, we simulate installing the rust pack we just created.
    pub async fn install_pack_from_url(&self, lang_name: &str, pack_url: &str) -> Result<(), LangManagerError> {
        // Here we would download a .tar.gz containing manifest.toml, queries/, and optionally download the WASM if it's not bundled.
        // For MVP, you'll need a way to distribute the pack.
        unimplemented!("Pack downloading is not fully implemented yet");
    }
}
