use regex::Regex;
use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tree_sitter::{Node, Parser, QueryCursor, Tree};

use super::chunking::smart_chunk_from_root;
use super::LANG_MANAGER;
use crate::models::chunk::SymbolKind;
use crate::models::{CodeChunk, ImportInfo, Symbol, SymbolReference};

/// Result of a single-pass file parse: symbols, chunks, references, and imports.
#[derive(Default)]
pub struct ParsedFile {
    pub symbols: Vec<Symbol>,
    pub chunks: Vec<CodeChunk>,
    pub refs: Vec<SymbolReference>,
    pub imports: Vec<ImportInfo>,
}

#[derive(Error, Debug)]
pub enum ParserError {
    #[error("Unsupported language: {0}")]
    UnsupportedLanguage(String),
    #[error("Parse error")]
    ParseError,
    #[error("Language manager error: {0}")]
    LangManagerError(String),
}

pub type Result<T> = std::result::Result<T, ParserError>;

/// Supported programming languages (wrapper around name for dynamic lookup)
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SupportedLanguage(pub String);

impl SupportedLanguage {
    pub const RUST: &'static str = "rust";
    pub const TYPESCRIPT: &'static str = "typescript";
    pub const JAVASCRIPT: &'static str = "javascript";
    pub const VUE: &'static str = "vue";
    pub const PYTHON: &'static str = "python";
    pub const GO: &'static str = "go";

    pub fn from_extension(ext: &str) -> Option<Self> {
        let name = match ext {
            "rs" => Self::RUST,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Self::TYPESCRIPT,
            "vue" => Self::VUE,
            "py" => Self::PYTHON,
            "go" => Self::GO,
            _ => return None,
        };
        Some(Self(name.to_string()))
    }

    pub fn name(&self) -> &str {
        &self.0
    }

    // Alias for name() to satisfy some parts of code
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SupportedLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Code parser using Tree-sitter
pub struct CodeParser {
    /// Cached tree from last parse (enables incremental re-parsing).
    old_tree: Option<Tree>,
}

impl CodeParser {
    pub fn new() -> Self {
        Self {
            old_tree: None,
        }
    }

    /// Extract script content from Vue SFC using tree-sitter-html for robust parsing.
    fn extract_vue_script(&self, content: &str) -> Option<(String, u32)> {
        if let Some(lang) = LANG_MANAGER.get_language("html") {
            let tree_opt = crate::indexer::parser::with_parser(|html_parser| {
                if html_parser.set_language(&lang.language).is_ok() {
                    html_parser.parse(content, None)
                } else {
                    None
                }
            });
            if let Some(tree) = tree_opt {
                    let root = tree.root_node();
                    for i in 0..root.child_count() {
                        if let Some(child) = root.child(i as u32) {
                            if child.kind() == "script_element" {
                                for j in 0..child.child_count() {
                                    if let Some(inner) = child.child(j as u32) {
                                    if inner.kind() == "raw_text" {
                                        let script_content =
                                            inner.utf8_text(content.as_bytes()).ok()?.to_string();
                                        let line_offset = inner.start_position().row as u32;
                                        return Some((script_content, line_offset));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Fallback: regex extraction
        let re = Regex::new(r"(?s)<script[^>]*>(.*?)</script>").ok()?;
        if let Some(captures) = re.captures(content) {
            if let Some(script_match) = captures.get(1) {
                let script_content = script_match.as_str().to_string();
                let before_script = &content[..script_match.start()];
                let line_offset = before_script.chars().filter(|&c| c == '\n').count() as u32;
                return Some((script_content, line_offset));
            }
        }
        None
    }

    pub fn parse_symbols(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<Symbol>> {
        if language.name() == SupportedLanguage::VUE {
            if let Some((script_content, line_offset)) = self.extract_vue_script(content) {
                let ts_lang = SupportedLanguage(SupportedLanguage::TYPESCRIPT.to_string());
                let mut symbols = self.parse_symbols_internal(&script_content, ts_lang)?;
                for symbol in &mut symbols {
                    symbol.line_start += line_offset as i32;
                    symbol.line_end += line_offset as i32;
                }
                return Ok(symbols);
            }
            return Ok(Vec::new());
        }

        self.parse_symbols_internal(content, language)
    }

    fn parse_symbols_internal(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<Symbol>> {
        let loaded_lang = LANG_MANAGER.get_language(language.name())
            .ok_or_else(|| ParserError::UnsupportedLanguage(language.name().to_string()))?;

        let tree = crate::indexer::parser::with_parser(|parser| {
            parser.set_language(&loaded_lang.language)
                .map_err(|e| ParserError::LangManagerError(e.to_string()))?;
            parser.parse(content, self.old_tree.as_ref())
                .ok_or(ParserError::ParseError)
        })?;

        let query = match loaded_lang.queries.get("symbols") {
            Some(q) => q,
            None => {
                tracing::warn!("No 'symbols' tree-sitter query found for {}", loaded_lang.manifest.language.name);
                return Ok(Vec::new());
            }
        };

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, tree.root_node(), content.as_bytes());
        while let Some(match_) = matches.next() {
            let mut name = String::new();
            let mut kind = String::new();
            let mut signature = None;
            let mut line_start = 0u32;
            let mut line_end = 0u32;

            for capture in match_.captures {
                let capture_name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = &content[node.byte_range()];

                match capture_name {
                    "name" => {
                        name = text.to_string();
                    }
                    "function" | "struct" | "enum" | "impl" | "trait" | "const" | "type" 
                    | "class" | "method" | "arrow" | "interface" | "var" | "static" | "module" | "macro"
                    | "decorated_function" | "decorated_class" | "typed_assignment"
                    | "type_declaration" | "method_declaration"
                    | "abstract_class" | "arrow_func" | "exported_function" | "exported_class" 
                    | "exported_arrow_func" | "declare_function" | "namespace" | "type_alias" => {
                        kind = match capture_name {
                            "decorated_function" | "exported_function" | "declare_function" => "function".to_string(),
                            "decorated_class" | "exported_class" | "abstract_class" => "class".to_string(),
                            "typed_assignment" | "var" => "variable".to_string(),
                            "arrow_func" | "exported_arrow_func" => "arrow".to_string(),
                            "type_declaration" => "type".to_string(),
                            "method_declaration" => "method".to_string(),
                            "type_alias" => "type".to_string(),
                            "namespace" | "module" => "module".to_string(),
                            _ => capture_name.to_string(),
                        };
                        line_start = node.start_position().row as u32;
                        line_end = node.end_position().row as u32;
                        // Capture the full signature (everything up to the body `{`)
                        // rather than just the first line. Multi-line signatures
                        // like `function foo(\n  a: A,\n  b: B\n): R {` used to
                        // store only `function foo(`, which surfaced as a
                        // truncated signature in `ts_get_signature`.
                        let body_node = node.child_by_field_name("body");
                        let sig_text = if let Some(body) = body_node {
                            let sig_end = body.start_byte().min(content.len());
                            let sig_start = node.start_byte();
                            if sig_end > sig_start {
                                &content[sig_start..sig_end]
                            } else {
                                text
                            }
                        } else {
                            text
                        };
                        // Collapse internal whitespace runs to single spaces so
                        // the signature stays one line in tool output without
                        // losing information.
                        let collapsed: String = sig_text
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ");
                        signature = Some(collapsed);
                    }
                    _ => {}
                }
            }

            if !name.is_empty() && !kind.is_empty() {
                symbols.push(Symbol {
                    id: 0,
                    file_id: 0,
                    name,
                    kind: SymbolKind::from_str(&kind),
                    line_start: line_start as i32,
                    line_end: line_end as i32,
                    signature,
                });
            }
        }

        self.old_tree = Some(tree.clone());
        if symbols.is_empty() {
            tracing::warn!(
                "AST parsed but 0 symbols extracted for {}. Root node: {}, child_count: {}, has_error: {}",
                loaded_lang.manifest.language.name,
                tree.root_node().kind(),
                tree.root_node().child_count(),
                tree.root_node().has_error()
            );
        }
        Ok(symbols)
    }

    pub fn parse_chunks(
        &mut self,
        content: &str,
        file_path: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<CodeChunk>> {
        if language.name() == SupportedLanguage::VUE {
            if let Some((script_content, line_offset)) = self.extract_vue_script(content) {
                let ts_lang = SupportedLanguage(SupportedLanguage::TYPESCRIPT.to_string());
                let mut chunks = self.parse_chunks_internal(
                    &script_content,
                    file_path,
                    ts_lang,
                )?;
                for chunk in &mut chunks {
                    chunk.line_start += line_offset;
                    chunk.line_end += line_offset;
                }
                return Ok(chunks);
            }
            return Ok(Vec::new());
        }

        self.parse_chunks_internal(content, file_path, language)
    }

    fn parse_chunks_internal(
        &mut self,
        content: &str,
        file_path: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<CodeChunk>> {
        let loaded_lang = LANG_MANAGER.get_language(language.name())
            .ok_or_else(|| ParserError::UnsupportedLanguage(language.name().to_string()))?;

        let tree = crate::indexer::parser::with_parser(|parser| {
            parser.set_language(&loaded_lang.language)
                .map_err(|e| ParserError::LangManagerError(e.to_string()))?;
            parser.parse(content, self.old_tree.as_ref())
                .ok_or(ParserError::ParseError)
        })?;

        let chunks = smart_chunk_from_root(tree.root_node(), content, file_path, language.clone())
            .map_err(|e| ParserError::LangManagerError(e.to_string()))?;
        self.old_tree = Some(tree);
        Ok(chunks)
    }

    pub fn parse_references(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<SymbolReference>> {
        if language.name() == SupportedLanguage::VUE {
            if let Some((script_content, line_offset)) = self.extract_vue_script(content) {
                let ts_lang = SupportedLanguage(SupportedLanguage::TYPESCRIPT.to_string());
                let mut refs = self.parse_references_internal(&script_content, ts_lang)?;
                for r in &mut refs {
                    r.line += line_offset as i32;
                }
                return Ok(refs);
            }
            return Ok(Vec::new());
        }

        self.parse_references_internal(content, language)
    }

    fn parse_references_internal(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<SymbolReference>> {
        let loaded_lang = LANG_MANAGER.get_language(language.name())
            .ok_or_else(|| ParserError::UnsupportedLanguage(language.name().to_string()))?;

        let tree = crate::indexer::parser::with_parser(|parser| {
            parser.set_language(&loaded_lang.language)
                .map_err(|e| ParserError::LangManagerError(e.to_string()))?;
            parser.parse(content, self.old_tree.as_ref())
                .ok_or(ParserError::ParseError)
        })?;

        let query = match loaded_lang.queries.get("references") {
            Some(q) => q,
            None => {
                tracing::warn!("No 'references' tree-sitter query found for {}", loaded_lang.manifest.language.name);
                return Ok(Vec::new());
            }
        };
        let mut cursor = QueryCursor::new();
        let mut refs = Vec::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, tree.root_node(), content.as_bytes());
        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = &content[node.byte_range()];

                if name == "call" || name == "type_usage" {
                    refs.push(SymbolReference {
                        id: 0,
                        source_symbol_id: 0,
                        target_name: text.to_string(),
                        target_symbol_id: None,
                        kind: "call".to_string(),
                        line: node.start_position().row as i32,
                    });
                }
            }
        }

        self.old_tree = Some(tree);
        Ok(refs)
    }

    pub fn parse_imports(&mut self, content: &str, language: SupportedLanguage) -> Vec<ImportInfo> {
        if language.name() == SupportedLanguage::VUE {
            if let Some((script, _)) = self.extract_vue_script(content) {
                let ts_lang = SupportedLanguage(SupportedLanguage::TYPESCRIPT.to_string());
                return self.parse_imports_internal(&script, ts_lang);
            }
            return Vec::new();
        }
        self.parse_imports_internal(content, language)
    }

    fn parse_imports_internal(&mut self, content: &str, language: SupportedLanguage) -> Vec<ImportInfo> {
        let loaded_lang = match LANG_MANAGER.get_language(language.name()) {
            Some(l) => l,
            None => return Vec::new(),
        };

        let tree = crate::indexer::parser::with_parser(|parser| {
            if parser.set_language(&loaded_lang.language).is_err() {
                return None;
            }
            parser.parse(content, None)
        });

        let tree = match tree {
            Some(t) => t,
            None => return Vec::new(),
        };

        let root = tree.root_node();
        let code = content;

        match language.name() {
            SupportedLanguage::RUST => Self::collect_rust_imports(root, code),
            SupportedLanguage::TYPESCRIPT | SupportedLanguage::JAVASCRIPT | SupportedLanguage::VUE => Self::collect_ts_imports(root, code),
            SupportedLanguage::PYTHON => Self::collect_python_imports(root, code),
            SupportedLanguage::GO => Self::collect_go_imports(root, code),
            _ => Vec::new(),
        }
    }

    fn collect_rust_imports(root: Node, code: &str) -> Vec<ImportInfo> {
        let mut imports = Vec::new();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() == "use_declaration" {
                let text = &code[node.byte_range()];
                imports.push(ImportInfo {
                    path: text.to_string(),
                    items: Vec::new(),
                    is_relative: false,
                    line: node.start_position().row as u32,
                });
            }
        }
        imports
    }

    fn collect_ts_imports(root: Node, code: &str) -> Vec<ImportInfo> {
        // Walk the AST and extract the actual module specifier (the string
        // literal after `from`) plus the imported items. The previous version
        // dumped the entire `import { foo } from './bar';` statement into
        // `path` and hard-coded `is_relative: false`, which is why
        // context_bundle's dependency resolver never matched anything for TS.
        let mut imports = Vec::new();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "import_statement" {
                continue;
            }
            let source_node = node.child_by_field_name("source");
            let source = match source_node {
                Some(n) => unquote_string_literal(&code[n.byte_range()]),
                None => continue,
            };
            let line = node.start_position().row as u32;
            let is_relative = source.starts_with('.') || source.starts_with('/');
            let items = extract_ts_imported_items(node, code);
            imports.push(ImportInfo {
                path: source,
                items,
                is_relative,
                line,
            });
        }
        imports
    }

    fn collect_python_imports(root: Node, code: &str) -> Vec<ImportInfo> {
        let mut imports = Vec::new();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() == "import_statement" || node.kind() == "import_from_statement" {
                let text = &code[node.byte_range()];
                imports.push(ImportInfo {
                    path: text.to_string(),
                    items: Vec::new(),
                    is_relative: false,
                    line: node.start_position().row as u32,
                });
            }
        }
        imports
    }

    fn collect_go_imports(root: Node, code: &str) -> Vec<ImportInfo> {
        let mut imports = Vec::new();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() == "import_declaration" {
                let text = &code[node.byte_range()];
                imports.push(ImportInfo {
                    path: text.to_string(),
                    items: Vec::new(),
                    is_relative: false,
                    line: node.start_position().row as u32,
                });
            }
        }
        imports
    }

    pub fn parse_file(
        &mut self,
        content: &str,
        file_path: &str,
        language: SupportedLanguage,
    ) -> Result<ParsedFile> {
        let mut result = ParsedFile::default();
        result.symbols = self.parse_symbols(content, language.clone())?;
        result.chunks = self.parse_chunks(content, file_path, language.clone())?;
        result.refs = self.parse_references(content, language.clone())?;
        result.imports = self.parse_imports(content, language);
        Ok(result)
    }
}

#[allow(dead_code)]
pub fn smart_chunk_file(
    content: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Result<Vec<CodeChunk>> {
    let mut parser = CodeParser::new();
    parser.parse_chunks(content, file_path, language)
}

/// Strip surrounding `'`, `"`, or backticks from a tree-sitter string-literal
/// node's text. tree-sitter returns the lexeme including quotes.
fn unquote_string_literal(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        let first = bytes[0];
        let last = bytes[trimmed.len() - 1];
        if (first == b'"' || first == b'\'' || first == b'`') && first == last {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }
    trimmed.to_string()
}

/// Extract the named/default imports from a TS `import_statement` node, e.g.
/// `import foo, { bar, baz } from "x"` → ["foo", "bar", "baz"]. Skip the
/// import_specifier `as <alias>` rename — we want the local binding name so
/// downstream tools can match references.
fn extract_ts_imported_items(node: tree_sitter::Node<'_>, code: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut clause_cursor = child.walk();
        for inner in child.named_children(&mut clause_cursor) {
            match inner.kind() {
                "identifier" => {
                    items.push(code[inner.byte_range()].to_string());
                }
                "named_imports" => {
                    let mut named_cursor = inner.walk();
                    for spec in inner.named_children(&mut named_cursor) {
                        if spec.kind() == "import_specifier" {
                            // Prefer the alias if present (`name as alias`),
                            // else the imported name.
                            let alias = spec.child_by_field_name("alias");
                            let name = spec.child_by_field_name("name");
                            if let Some(n) = alias.or(name) {
                                items.push(code[n.byte_range()].to_string());
                            }
                        }
                    }
                }
                "namespace_import" => {
                    let mut ns_cursor = inner.walk();
                    for inner2 in inner.named_children(&mut ns_cursor) {
                        if inner2.kind() == "identifier" {
                            items.push(code[inner2.byte_range()].to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_wasm_parsing() {
        let mut parser = CodeParser::new();
        let code = "fn main() { println!(\"hello\"); }";
        let lang = SupportedLanguage(SupportedLanguage::RUST.to_string());
        
        match parser.parse_file(code, "test.rs", lang.clone()) {
            Ok(parsed) => {
                println!("--- DEBUG WASM PARSING ---");
                println!("Symbols: {}", parsed.symbols.len());
                println!("Chunks: {}", parsed.chunks.len());
                println!("Refs: {}", parsed.refs.len());
                
                if parsed.symbols.is_empty() {
                    println!("WARNING: AST parsed but 0 symbols extracted. Query mismatch or empty AST?");
                    
                    if let Some(lang_obj) = LANG_MANAGER.get_language(lang.name()) {
                        let tree = crate::indexer::parser::with_parser(|ts_parser| {
                            ts_parser.set_language(&lang_obj.language).unwrap();
                            ts_parser.parse(code, None).unwrap()
                        });
                        let root = tree.root_node();
                        println!("AST root node kind: {}", root.kind());
                        println!("AST child count: {}", root.child_count());
                        println!("AST root sexp: {}", root.to_sexp());
                        
                        if let Some(query) = lang_obj.queries.get("symbols") {
                            let mut cursor = tree_sitter::QueryCursor::new();
                            let matches = cursor.matches(query, root, code.as_bytes());
                            println!("Query matches count: {}", matches.count());
                        } else {
                            println!("No symbols query found!");
                        }
                    }
                }
            }
            Err(e) => {
                println!("Parse failed: {:?}", e);
            }
        }
    }
}
