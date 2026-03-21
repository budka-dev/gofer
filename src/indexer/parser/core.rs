use regex::Regex;
use std::sync::OnceLock;
use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Tree};

use super::chunking::smart_chunk_from_root;
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
}

pub type Result<T> = std::result::Result<T, ParserError>;

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
    TypeScript,
    JavaScript,
    Vue,
    Python,
    Go,
}

impl SupportedLanguage {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" => Some(Self::JavaScript),
            "vue" => Some(Self::Vue),
            "py" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub(crate) fn tree_sitter_language(&self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript | Self::JavaScript | Self::Vue => {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            }
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    pub(crate) fn symbol_query(&self) -> &'static Query {
        static RUST: OnceLock<Query> = OnceLock::new();
        static TS: OnceLock<Query> = OnceLock::new();
        static VUE: OnceLock<Query> = OnceLock::new();
        static PY: OnceLock<Query> = OnceLock::new();
        static GO: OnceLock<Query> = OnceLock::new();

        let (lock, lang, query_str) = match self {
            Self::Rust => (&RUST, self.tree_sitter_language(), RUST_QUERY),
            Self::TypeScript | Self::JavaScript => {
                (&TS, self.tree_sitter_language(), TYPESCRIPT_QUERY)
            }
            Self::Vue => (&VUE, self.tree_sitter_language(), VUE_QUERY),
            Self::Python => (&PY, self.tree_sitter_language(), PYTHON_QUERY),
            Self::Go => (&GO, self.tree_sitter_language(), GO_QUERY),
        };

        lock.get_or_init(|| {
            Query::new(&lang, query_str).expect("Failed to compile tree-sitter symbol query")
        })
    }

    pub(crate) fn refs_query(&self) -> &'static Query {
        static RUST: OnceLock<Query> = OnceLock::new();
        static TS: OnceLock<Query> = OnceLock::new();
        static VUE: OnceLock<Query> = OnceLock::new();
        static PY: OnceLock<Query> = OnceLock::new();
        static GO: OnceLock<Query> = OnceLock::new();

        let (lock, lang, query_str) = match self {
            Self::Rust => (&RUST, self.tree_sitter_language(), RUST_REFS_QUERY),
            Self::TypeScript | Self::JavaScript => {
                (&TS, self.tree_sitter_language(), TYPESCRIPT_REFS_QUERY)
            }
            Self::Vue => (&VUE, self.tree_sitter_language(), VUE_REFS_QUERY),
            Self::Python => (&PY, self.tree_sitter_language(), PYTHON_REFS_QUERY),
            Self::Go => (&GO, self.tree_sitter_language(), GO_REFS_QUERY),
        };

        lock.get_or_init(|| {
            Query::new(&lang, query_str).expect("Failed to compile tree-sitter refs query")
        })
    }
}

// Tree-sitter query for Rust symbols
const RUST_QUERY: &str = r#"
(function_item
  name: (identifier) @name
) @function

(struct_item
  name: (type_identifier) @name
) @struct

(enum_item
  name: (type_identifier) @name
) @enum

(impl_item
  type: (type_identifier) @name
) @impl

(impl_item
  type: (generic_type
    type: (type_identifier) @name
  )
) @impl

(trait_item
  name: (type_identifier) @name
) @trait

(const_item
  name: (identifier) @name
) @const

(static_item
  name: (identifier) @name
) @static

(mod_item
  name: (identifier) @name
) @module

(macro_definition
  name: (identifier) @name
) @macro

(type_item
  name: (type_identifier) @name
) @type_alias
"#;

// Tree-sitter query for Rust references (calls, uses, type mentions)
const RUST_REFS_QUERY: &str = r#"
(call_expression
  function: (identifier) @call
)

(call_expression
  function: (field_expression
    field: (field_identifier) @call
  )
)

(call_expression
  function: (scoped_identifier
    name: (identifier) @call
  )
)

(macro_invocation
  macro: (identifier) @call
)

(use_declaration
  argument: (scoped_identifier
    name: (identifier) @import
  )
)

(use_declaration
  argument: (identifier) @import
)

(type_identifier) @type_usage
"#;

// Tree-sitter query for TypeScript/JavaScript symbols
const TYPESCRIPT_QUERY: &str = r#"
(function_declaration
  name: (identifier) @name
) @function

(class_declaration
  name: (type_identifier) @name
) @class

(abstract_class_declaration
  name: (type_identifier) @name
) @abstract_class

(method_definition
  name: (property_identifier) @name
) @method

(interface_declaration
  name: (type_identifier) @name
) @interface

(type_alias_declaration
  name: (type_identifier) @name
) @type_alias

(enum_declaration
  name: (identifier) @name
) @enum

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (arrow_function)
  )
) @arrow_func

(export_statement
  declaration: (function_declaration
    name: (identifier) @name
  )
) @exported_function

(export_statement
  declaration: (class_declaration
    name: (type_identifier) @name
  )
) @exported_class

(export_statement
  declaration: (lexical_declaration
    (variable_declarator
      name: (identifier) @name
      value: (arrow_function)
    )
  )
) @exported_arrow_func

(ambient_declaration
  (function_signature
    name: (identifier) @name
  )
) @declare_function

(module
  name: (string) @name
) @namespace
"#;

// Tree-sitter query for TypeScript/JavaScript references
const TYPESCRIPT_REFS_QUERY: &str = r#"
(call_expression
  function: (identifier) @call
)

(call_expression
  function: (member_expression
    property: (property_identifier) @call
  )
)

(new_expression
  constructor: (identifier) @call
)

(import_specifier
  name: (identifier) @import
)

(import_clause
  (identifier) @import
)

(type_identifier) @type_usage
"#;

// Tree-sitter query for Vue SFC - extracts script element
const VUE_QUERY: &str = r#"
(script_element) @script
"#;

// Vue refs query (we'll parse the script content with TypeScript parser)
const VUE_REFS_QUERY: &str = r#"
(script_element) @script
"#;

// Tree-sitter query for Python symbols
const PYTHON_QUERY: &str = r#"
(function_definition
  name: (identifier) @name
) @function

(class_definition
  name: (identifier) @name
) @class

(decorated_definition
  definition: (function_definition
    name: (identifier) @name
  )
) @decorated_function

(decorated_definition
  definition: (class_definition
    name: (identifier) @name
  )
) @decorated_class

(assignment
  left: (identifier) @name
  type: (type) @_type
) @typed_assignment
"#;

// Tree-sitter query for Python references
const PYTHON_REFS_QUERY: &str = r#"
(call
  function: (identifier) @call
)

(call
  function: (attribute
    attribute: (identifier) @call
  )
)

(import_from_statement
  name: (dotted_name) @import
)

(import_statement
  name: (dotted_name) @import
)

(aliased_import
  name: (dotted_name) @import
)

(type) @type_usage
"#;

// Tree-sitter query for Go symbols
const GO_QUERY: &str = r#"
(function_declaration
  name: (identifier) @name
) @function

(method_declaration
  name: (field_identifier) @name
) @method

(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (struct_type)
  )
) @struct

(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (interface_type)
  )
) @interface

(const_declaration
  (const_spec
    name: (identifier) @name
  )
) @const

(var_declaration
  (var_spec
    name: (identifier) @name
  )
) @var
"#;

// Tree-sitter query for Go references
const GO_REFS_QUERY: &str = r#"
(call_expression
  function: (identifier) @call
)

(call_expression
  function: (selector_expression
    field: (field_identifier) @call
  )
)

(composite_literal
  type: (type_identifier) @call
)

(import_spec
  path: (interpreted_string_literal) @import
)

(type_identifier) @type_usage
"#;

/// Code parser using Tree-sitter
pub struct CodeParser {
    parser: Parser,
    /// Cached tree from last parse (enables incremental re-parsing).
    old_tree: Option<Tree>,
}

impl CodeParser {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            old_tree: None,
        }
    }

    /// Reset cached tree (call when switching to a different file).
    /// Extract script content from Vue SFC using tree-sitter-html for robust parsing.
    /// Falls back to regex if tree-sitter parsing fails.
    fn extract_vue_script(&self, content: &str) -> Option<(String, u32)> {
        // Try tree-sitter-html first for robust extraction
        let mut html_parser = Parser::new();
        let html_lang: Language = tree_sitter_html::LANGUAGE.into();
        if html_parser.set_language(&html_lang).is_ok() {
            if let Some(tree) = html_parser.parse(content, None) {
                let root = tree.root_node();
                // Walk children looking for <script> element
                for i in 0..root.child_count() {
                    if let Some(child) = root.child(i as u32) {
                        if child.kind() == "script_element" {
                            // Find the raw_text child (script body)
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

    /// Parse a file and extract symbols
    pub fn parse_symbols(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<Symbol>> {
        // For Vue files, extract script and parse as TypeScript
        if language == SupportedLanguage::Vue {
            if let Some((script_content, line_offset)) = self.extract_vue_script(content) {
                let mut symbols =
                    self.parse_symbols_internal(&script_content, SupportedLanguage::TypeScript)?;
                // Adjust line numbers to account for script position in Vue file
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
        self.parser
            .set_language(&language.tree_sitter_language())
            .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

        let tree = self
            .parser
            .parse(content, self.old_tree.as_ref())
            .ok_or(ParserError::ParseError)?;

        let query = language.symbol_query();

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
                    // Core symbol types
                    "function" | "struct" | "enum" | "impl" | "trait" | "const" | "type" 
                    | "class" | "method" | "arrow" | "interface" | "var" | "static" | "module" | "macro"
                    // Python decorated
                    | "decorated_function" | "decorated_class" | "typed_assignment"
                    // Go specific
                    | "type_declaration" | "method_declaration"
                    // TypeScript/JS extended
                    | "abstract_class" | "arrow_func" | "exported_function" | "exported_class" 
                    | "exported_arrow_func" | "declare_function" | "namespace" | "type_alias" => {
                        // Normalize kind names to canonical forms
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
                        let first_line = text.lines().next().unwrap_or("");
                        signature = Some(first_line.to_string());
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

        // Cache tree for incremental re-parsing
        self.old_tree = Some(tree);

        Ok(symbols)
    }

    /// Parse a file and extract code chunks for embedding
    pub fn parse_chunks(
        &mut self,
        content: &str,
        file_path: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<CodeChunk>> {
        // For Vue files, extract script and parse as TypeScript
        if language == SupportedLanguage::Vue {
            if let Some((script_content, line_offset)) = self.extract_vue_script(content) {
                let mut chunks = self.parse_chunks_internal(
                    &script_content,
                    file_path,
                    SupportedLanguage::TypeScript,
                )?;
                // Adjust line numbers
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
        self.parser
            .set_language(&language.tree_sitter_language())
            .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

        let tree = self
            .parser
            .parse(content, self.old_tree.as_ref())
            .ok_or(ParserError::ParseError)?;

        let query = language.symbol_query();

        let mut cursor = QueryCursor::new();
        let mut chunks = Vec::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, tree.root_node(), content.as_bytes());
        while let Some(match_) = matches.next() {
            let mut name = String::new();
            let mut kind = String::new();
            let mut chunk_content = String::new();
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
                    // Core symbol types
                    "function" | "struct" | "enum" | "impl" | "trait" | "const" | "type"
                    | "class" | "method" | "arrow" | "interface" | "var" | "static" | "module" | "macro"
                    // Python decorated
                    | "decorated_function" | "decorated_class" | "typed_assignment"
                    // Go specific
                    | "type_declaration" | "method_declaration"
                    // TypeScript/JS extended
                    | "abstract_class" | "arrow_func" | "exported_function" | "exported_class" 
                    | "exported_arrow_func" | "declare_function" | "namespace" | "type_alias" => {
                        // Normalize kind for chunk metadata
                        kind = match capture_name {
                            "decorated_function" | "exported_function" | "declare_function" => "function".to_string(),
                            "decorated_class" | "exported_class" | "abstract_class" => "class".to_string(),
                            "typed_assignment" | "var" => "variable".to_string(),
                            "arrow_func" | "exported_arrow_func" => "arrow".to_string(),
                            "type_declaration" | "type_alias" => "type".to_string(),
                            "method_declaration" => "method".to_string(),
                            "namespace" | "module" => "module".to_string(),
                            _ => capture_name.to_string(),
                        };
                        line_start = node.start_position().row as u32;
                        line_end = node.end_position().row as u32;
                        chunk_content = text.to_string();
                    }
                    _ => {}
                }
            }

            if !chunk_content.is_empty() {
                let id = format!("{}:{}:{}", file_path, line_start, line_end);
                chunks.push(CodeChunk {
                    id,
                    file_path: file_path.to_string(),
                    content: chunk_content,
                    line_start,
                    line_end,
                    symbol_name: if name.is_empty() { None } else { Some(name) },
                    symbol_kind: if kind.is_empty() {
                        None
                    } else {
                        Some(SymbolKind::from_str(&kind))
                    },
                    symbol_path: None,
                    scopes: Vec::new(),
                });
            }
        }

        Ok(chunks)
    }

    /// Parse a file and extract references (function calls, imports, type usages)
    pub fn parse_references(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<SymbolReference>> {
        // For Vue files, extract script and parse as TypeScript
        if language == SupportedLanguage::Vue {
            let mut all_refs = Vec::new();

            // Script references
            if let Some((script_content, line_offset)) = self.extract_vue_script(content) {
                let mut refs =
                    self.parse_references_internal(&script_content, SupportedLanguage::TypeScript)?;
                for r in &mut refs {
                    r.line += line_offset as i32;
                }
                all_refs.extend(refs);
            }

            // Template component references
            all_refs.extend(self.extract_vue_template_refs(content));

            return Ok(all_refs);
        }

        self.parse_references_internal(content, language)
    }

    /// Extract component usage references from Vue <template> using tree-sitter-html.
    /// Detects PascalCase and kebab-case custom element tags.
    fn extract_vue_template_refs(&self, content: &str) -> Vec<SymbolReference> {
        let mut refs = Vec::new();

        let mut html_parser = Parser::new();
        let html_lang: Language = tree_sitter_html::LANGUAGE.into();
        if html_parser.set_language(&html_lang).is_err() {
            return refs;
        }

        let Some(tree) = html_parser.parse(content, None) else {
            return refs;
        };

        // Walk the tree looking for element nodes with custom component names
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "start_tag" || node.kind() == "self_closing_tag" {
                // First child of a tag is the tag_name
                if let Some(tag_name_node) = node.child(1 as u32) {
                    if tag_name_node.kind() == "tag_name" {
                        if let Ok(tag_name) = tag_name_node.utf8_text(content.as_bytes()) {
                            // Custom components: PascalCase or contains hyphen (kebab-case)
                            let is_custom = tag_name.contains('-')
                                || (tag_name.len() > 1
                                    && tag_name.chars().next().is_some_and(|c| c.is_uppercase()));

                            if is_custom {
                                refs.push(SymbolReference {
                                    id: 0,
                                    source_symbol_id: 0,
                                    target_name: tag_name.to_string(),
                                    target_symbol_id: None,
                                    kind: "component_usage".to_string(),
                                    line: tag_name_node.start_position().row as i32 + 1,
                                });
                            }
                        }
                    }
                }
            }

            // Traverse children
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }
        }

        refs
    }

    fn parse_references_internal(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Result<Vec<SymbolReference>> {
        self.parser
            .set_language(&language.tree_sitter_language())
            .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

        let tree = self
            .parser
            .parse(content, None)
            .ok_or(ParserError::ParseError)?;

        let query = language.refs_query();

        let mut cursor = QueryCursor::new();
        let mut refs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, tree.root_node(), content.as_bytes());
        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let capture_name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = &content[node.byte_range()];
                let line = node.start_position().row as i32;

                // Skip common primitives and keywords
                if is_primitive_or_keyword(text) {
                    continue;
                }

                let kind = match capture_name {
                    "call" => "call",
                    "import" => "import",
                    "type_usage" => "type_usage",
                    _ => continue,
                };

                // Deduplicate by (name, line, kind)
                let key = (text.to_string(), line, kind);
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);

                refs.push(SymbolReference {
                    id: 0,
                    source_symbol_id: 0,
                    target_name: text.to_string(),
                    target_symbol_id: None,
                    kind: kind.to_string(),
                    line,
                });
            }
        }

        Ok(refs)
    }

    /// Parse imports from a file using Tree-sitter AST (not regex).
    pub fn parse_imports(&mut self, content: &str, language: SupportedLanguage) -> Vec<ImportInfo> {
        match language {
            SupportedLanguage::Vue => {
                if let Some((script, line_offset)) = self.extract_vue_script(content) {
                    let mut imports =
                        self.parse_imports_internal(&script, SupportedLanguage::TypeScript);
                    for imp in &mut imports {
                        imp.line += line_offset;
                    }
                    imports
                } else {
                    Vec::new()
                }
            }
            _ => self.parse_imports_internal(content, language),
        }
    }

    fn parse_imports_internal(
        &mut self,
        content: &str,
        language: SupportedLanguage,
    ) -> Vec<ImportInfo> {
        let lang = language.tree_sitter_language();
        if self.parser.set_language(&lang).is_err() {
            return Vec::new();
        }

        let tree = match self.parser.parse(content, self.old_tree.as_ref()) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let root = tree.root_node();
        let code = content.as_bytes();

        match language {
            SupportedLanguage::Rust => Self::collect_rust_imports(root, code),
            SupportedLanguage::TypeScript
            | SupportedLanguage::JavaScript
            | SupportedLanguage::Vue => Self::collect_ts_imports(root, code),
            SupportedLanguage::Python => Self::collect_python_imports(root, code),
            SupportedLanguage::Go => Self::collect_go_imports(root, code),
        }
    }

    // -- Rust: use_declaration ---------------------------------------------------

    fn collect_rust_imports(root: Node<'_>, code: &[u8]) -> Vec<ImportInfo> {
        let mut imports = Vec::new();
        for i in 0..root.child_count() {
            let child = match root.child(i as u32) {
                Some(c) => c,
                None => continue,
            };
            // Match both `use ...;` and `pub use ...;`
            if child.kind() != "use_declaration" {
                continue;
            }
            let line = child.start_position().row as u32;
            // The argument subtree sits under "argument" field
            if let Some(arg) = child.child_by_field_name("argument") {
                let mut prefix = Vec::new();
                Self::walk_rust_use(arg, code, &mut prefix, line, &mut imports);
            }
        }
        imports
    }

    /// Recursively walk a Rust `use` argument tree, collecting (path, items).
    fn walk_rust_use(
        node: Node<'_>,
        code: &[u8],
        prefix: &mut Vec<String>,
        line: u32,
        out: &mut Vec<ImportInfo>,
    ) {
        match node.kind() {
            // Simple: `use foo;`
            "identifier" | "crate" | "self" | "super" => {
                let text = node.utf8_text(code).unwrap_or("");
                let mut path_parts = prefix.clone();
                path_parts.push(text.to_string());
                let full = path_parts.join("::");
                if !full.starts_with("std::") && !full.starts_with("core::") {
                    let item = text.to_string();
                    let is_rel = full.starts_with("crate")
                        || full.starts_with("super")
                        || full.starts_with("self");
                    out.push(ImportInfo {
                        path: full,
                        items: vec![item],
                        is_relative: is_rel,
                        line,
                    });
                }
            }
            // `use foo::bar;`
            "scoped_identifier" => {
                let path_node = node.child_by_field_name("path");
                let name_node = node.child_by_field_name("name");
                let full = node.utf8_text(code).unwrap_or("").to_string();
                if !full.starts_with("std::") && !full.starts_with("core::") {
                    let item = name_node
                        .and_then(|n| n.utf8_text(code).ok())
                        .unwrap_or_else(|| {
                            path_node.and_then(|n| n.utf8_text(code).ok()).unwrap_or("")
                        });
                    let is_rel = full.starts_with("crate")
                        || full.starts_with("super")
                        || full.starts_with("self");
                    out.push(ImportInfo {
                        path: full,
                        items: vec![item.to_string()],
                        is_relative: is_rel,
                        line,
                    });
                }
            }
            // `use foo::{A, B};`
            "scoped_use_list" => {
                // path is the prefix, list contains the items
                let path_node = node.child_by_field_name("path");
                let list_node = node.child_by_field_name("list");
                let path_text = path_node.and_then(|n| n.utf8_text(code).ok()).unwrap_or("");
                let mut new_prefix = prefix.clone();
                if !path_text.is_empty() {
                    new_prefix.extend(path_text.split("::").map(|s| s.to_string()));
                }
                if let Some(list) = list_node {
                    Self::walk_rust_use(list, code, &mut new_prefix, line, out);
                }
            }
            // The list `{A, B, C}` inside scoped_use_list
            "use_list" => {
                for j in 0..node.child_count() {
                    if let Some(child) = node.child(j as u32) {
                        if child.is_named() {
                            Self::walk_rust_use(child, code, prefix, line, out);
                        }
                    }
                }
            }
            // `use foo as bar;`
            "use_as_clause" => {
                // The first named child is the original name
                if let Some(orig) = node.named_child(0 as u32) {
                    Self::walk_rust_use(orig, code, prefix, line, out);
                }
            }
            // `use foo::*;`
            "use_wildcard" => {
                if let Some(path_node) = node.child_by_field_name("path") {
                    let full = path_node.utf8_text(code).unwrap_or("").to_string();
                    if !full.starts_with("std::") && !full.starts_with("core::") {
                        let is_rel = full.starts_with("crate")
                            || full.starts_with("super")
                            || full.starts_with("self");
                        out.push(ImportInfo {
                            path: format!("{}::*", full),
                            items: vec!["*".to_string()],
                            is_relative: is_rel,
                            line,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // -- TypeScript / JavaScript: import_statement --------------------------------

    fn collect_ts_imports(root: Node<'_>, code: &[u8]) -> Vec<ImportInfo> {
        let mut imports = Vec::new();
        for i in 0..root.child_count() {
            let child = match root.child(i as u32) {
                Some(c) => c,
                None => continue,
            };
            // import_statement  — also catches `import type { X } from 'y'`
            if child.kind() != "import_statement" {
                continue;
            }
            let line = child.start_position().row as u32;

            // source: the string literal (module specifier)
            let source = match child.child_by_field_name("source") {
                Some(s) => {
                    let raw = s.utf8_text(code).unwrap_or("");
                    raw.trim_matches(|c| c == '\'' || c == '"').to_string()
                }
                None => continue,
            };

            let is_relative = source.starts_with('.') || source.starts_with("@/");
            let mut items = Vec::new();

            // Walk children to find import_clause / named_imports / namespace_import
            Self::walk_ts_import_clause(child, code, &mut items);

            // Side-effect import: `import './foo'` — no items
            if items.is_empty() {
                items.push("*".to_string());
            }

            imports.push(ImportInfo {
                path: source,
                items,
                is_relative,
                line,
            });
        }
        imports
    }

    fn walk_ts_import_clause(node: Node<'_>, code: &[u8], items: &mut Vec<String>) {
        for j in 0..node.child_count() {
            let child = match node.child(j as u32) {
                Some(c) => c,
                None => continue,
            };
            match child.kind() {
                // default import: `import Foo from '...'`
                "identifier" if node.kind() == "import_clause" => {
                    if let Ok(t) = child.utf8_text(code) {
                        items.push(t.to_string());
                    }
                }
                // `import * as ns from '...'`
                "namespace_import" => {
                    // the alias identifier
                    for k in 0..child.child_count() {
                        if let Some(id) = child.child(k as u32) {
                            if id.kind() == "identifier" {
                                if let Ok(t) = id.utf8_text(code) {
                                    items.push(t.to_string());
                                }
                            }
                        }
                    }
                }
                // `import { A, B as C } from '...'`
                "named_imports" => {
                    for k in 0..child.named_child_count() {
                        if let Some(spec) = child.named_child(k as u32) {
                            // import_specifier has name: and (optional) alias:
                            let name = spec
                                .child_by_field_name("name")
                                .and_then(|n| n.utf8_text(code).ok())
                                .unwrap_or_else(|| spec.utf8_text(code).unwrap_or(""));
                            if !name.is_empty() {
                                items.push(name.to_string());
                            }
                        }
                    }
                }
                // recurse into import_clause
                "import_clause" => Self::walk_ts_import_clause(child, code, items),
                _ => {}
            }
        }
    }

    // -- Python: import_statement / import_from_statement --------------------------

    fn collect_python_imports(root: Node<'_>, code: &[u8]) -> Vec<ImportInfo> {
        let mut imports = Vec::new();
        Self::walk_python_imports(root, code, &mut imports);
        imports
    }

    fn walk_python_imports(node: Node<'_>, code: &[u8], out: &mut Vec<ImportInfo>) {
        for i in 0..node.child_count() {
            let child = match node.child(i as u32) {
                Some(c) => c,
                None => continue,
            };
            match child.kind() {
                "import_statement" => {
                    // `import foo`, `import foo.bar`, `import foo as f`
                    let line = child.start_position().row as u32;
                    for j in 0..child.named_child_count() {
                        if let Some(name_node) = child.named_child(j as u32) {
                            let raw = name_node.utf8_text(code).unwrap_or("");
                            // aliased_import: `import foo as f` — take the dotted_name
                            let path = if name_node.kind() == "aliased_import" {
                                name_node
                                    .child_by_field_name("name")
                                    .and_then(|n| n.utf8_text(code).ok())
                                    .unwrap_or(raw)
                            } else {
                                raw
                            };
                            if !path.is_empty() {
                                let item = path.rsplit('.').next().unwrap_or(path).to_string();
                                out.push(ImportInfo {
                                    path: path.to_string(),
                                    items: vec![item],
                                    is_relative: false,
                                    line,
                                });
                            }
                        }
                    }
                }
                "import_from_statement" => {
                    // `from foo import A, B` / `from . import x` / `from ...pkg import *`
                    let line = child.start_position().row as u32;

                    // module_name: can be dotted_name or relative_import
                    let module = child
                        .child_by_field_name("module_name")
                        .and_then(|n| n.utf8_text(code).ok())
                        .unwrap_or("");

                    let is_relative = module.starts_with('.');

                    let mut items = Vec::new();
                    // Collect imported names after "import"
                    for j in 0..child.named_child_count() {
                        if let Some(n) = child.named_child(j as u32) {
                            match n.kind() {
                                "dotted_name" | "identifier" => {
                                    // Skip the module_name node (it appears before "import")
                                    if n.start_byte()
                                        > child
                                            .child_by_field_name("module_name")
                                            .map(|m| m.end_byte())
                                            .unwrap_or(0)
                                    {
                                        let t = n.utf8_text(code).unwrap_or("");
                                        if !t.is_empty() {
                                            items.push(t.to_string());
                                        }
                                    }
                                }
                                "aliased_import" => {
                                    let name = n
                                        .child_by_field_name("name")
                                        .and_then(|nm| nm.utf8_text(code).ok())
                                        .unwrap_or("");
                                    if !name.is_empty() {
                                        items.push(name.to_string());
                                    }
                                }
                                "wildcard_import" => {
                                    items.push("*".to_string());
                                }
                                _ => {}
                            }
                        }
                    }

                    if !module.is_empty() || !items.is_empty() {
                        out.push(ImportInfo {
                            path: module.to_string(),
                            items,
                            is_relative,
                            line,
                        });
                    }
                }
                // Recurse into decorated_definition, if_statement, etc. where imports may appear
                _ if child.named_child_count() > 0
                    && matches!(
                        child.kind(),
                        "if_statement"
                            | "try_statement"
                            | "block"
                            | "except_clause"
                            | "with_statement"
                            | "function_definition"
                            | "module"
                    ) =>
                {
                    Self::walk_python_imports(child, code, out);
                }
                _ => {}
            }
        }
    }

    // -- Go: import_declaration ---------------------------------------------------

    fn collect_go_imports(root: Node<'_>, code: &[u8]) -> Vec<ImportInfo> {
        let mut imports = Vec::new();

        // Go source_file has top-level import_declaration nodes
        for i in 0..root.child_count() {
            let child = match root.child(i as u32) {
                Some(c) => c,
                None => continue,
            };
            if child.kind() != "import_declaration" {
                continue;
            }
            // Single import: `import "fmt"` or grouped: `import ( "fmt" \n "os" )`
            for j in 0..child.named_child_count() {
                if let Some(spec) = child.named_child(j as u32) {
                    match spec.kind() {
                        "import_spec" => {
                            Self::push_go_import_spec(spec, code, &mut imports);
                        }
                        "import_spec_list" => {
                            for k in 0..spec.named_child_count() {
                                if let Some(inner) = spec.named_child(k as u32) {
                                    if inner.kind() == "import_spec" {
                                        Self::push_go_import_spec(inner, code, &mut imports);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        imports
    }

    fn push_go_import_spec(spec: Node<'_>, code: &[u8], out: &mut Vec<ImportInfo>) {
        let path_node = match spec.child_by_field_name("path") {
            Some(p) => p,
            None => return,
        };
        let raw = path_node.utf8_text(code).unwrap_or("");
        let path = raw.trim_matches('"');
        if path.is_empty() {
            return;
        }
        let pkg = path.rsplit('/').next().unwrap_or(path).to_string();
        let line = spec.start_position().row as u32;

        out.push(ImportInfo {
            path: path.to_string(),
            items: vec![pkg],
            is_relative: path.starts_with('.'),
            line,
        });
    }

    // === parse_file: single-pass extraction of symbols, chunks, refs, imports ===

    /// Parse a file once and extract all data in a single pass.
    /// Eliminates redundant tree-sitter parse calls vs calling each method individually.
    pub fn parse_file(
        &mut self,
        content: &str,
        file_path: &str,
        language: SupportedLanguage,
    ) -> Result<ParsedFile> {
        // Vue: extract script, parse as TypeScript, adjust line offsets
        if language == SupportedLanguage::Vue {
            return self.parse_vue_file(content, file_path);
        }

        let lang = language.tree_sitter_language();
        self.parser
            .set_language(&lang)
            .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

        // === Single parse ===
        let tree = self
            .parser
            .parse(content, self.old_tree.as_ref())
            .ok_or(ParserError::ParseError)?;
        let root = tree.root_node();
        let code = content.as_bytes();

        // 1. Symbols — via symbol query
        let sym_query = language.symbol_query();
        let symbols = Self::extract_symbols_from_tree(root, content, sym_query);

        // 2. Smart chunks from tree walk (preferred), fallback to query-based chunks
        let chunks =
            smart_chunk_from_root(root, content, file_path, language).unwrap_or_else(|_| {
                Self::extract_chunks_from_tree(root, content, file_path, sym_query)
            });

        // 3. References — via refs query
        let refs_query = language.refs_query();
        let refs = Self::extract_refs_from_tree(root, content, refs_query);

        // 4. Imports — via tree walk
        let imports = match language {
            SupportedLanguage::Rust => Self::collect_rust_imports(root, code),
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
                Self::collect_ts_imports(root, code)
            }
            SupportedLanguage::Python => Self::collect_python_imports(root, code),
            SupportedLanguage::Go => Self::collect_go_imports(root, code),
            SupportedLanguage::Vue => Vec::new(), // handled above
        };

        Ok(ParsedFile {
            symbols,
            chunks,
            refs,
            imports,
        })
    }

    fn parse_vue_file(&mut self, content: &str, file_path: &str) -> Result<ParsedFile> {
        let Some((script, line_offset)) = self.extract_vue_script(content) else {
            return Ok(ParsedFile {
                symbols: Vec::new(),
                chunks: Vec::new(),
                refs: Vec::new(),
                imports: Vec::new(),
            });
        };

        let mut result = self.parse_file(&script, file_path, SupportedLanguage::TypeScript)?;

        // Adjust line numbers
        for s in &mut result.symbols {
            s.line_start += line_offset as i32;
            s.line_end += line_offset as i32;
        }
        for c in &mut result.chunks {
            c.line_start += line_offset;
            c.line_end += line_offset;
            c.id = format!("{}:{}:{}", file_path, c.line_start, c.line_end);
        }
        for r in &mut result.refs {
            r.line += line_offset as i32;
        }
        for i in &mut result.imports {
            i.line += line_offset;
        }

        // Add template component refs
        result.refs.extend(self.extract_vue_template_refs(content));

        Ok(result)
    }

    // --- Static extractors that work on a pre-parsed tree ---

    fn extract_symbols_from_tree(root: Node<'_>, content: &str, query: &Query) -> Vec<Symbol> {
        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, root, content.as_bytes());
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
                    | "class" | "method" | "arrow" | "interface" => {
                        kind = capture_name.to_string();
                        line_start = node.start_position().row as u32;
                        line_end = node.end_position().row as u32;
                        let first_line = text.lines().next().unwrap_or("");
                        signature = Some(first_line.to_string());
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

        symbols
    }

    fn extract_chunks_from_tree(
        root: Node<'_>,
        content: &str,
        file_path: &str,
        query: &Query,
    ) -> Vec<CodeChunk> {
        let mut cursor = QueryCursor::new();
        let mut chunks = Vec::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, root, content.as_bytes());
        while let Some(match_) = matches.next() {
            let mut name = String::new();
            let mut kind = String::new();
            let mut chunk_content = String::new();
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
                    | "class" | "method" | "arrow" | "interface" => {
                        kind = capture_name.to_string();
                        line_start = node.start_position().row as u32;
                        line_end = node.end_position().row as u32;
                        chunk_content = text.to_string();
                    }
                    _ => {}
                }
            }

            if !chunk_content.is_empty() {
                let id = format!("{}:{}:{}", file_path, line_start, line_end);
                chunks.push(CodeChunk {
                    id,
                    file_path: file_path.to_string(),
                    content: chunk_content,
                    line_start,
                    line_end,
                    symbol_name: if name.is_empty() { None } else { Some(name) },
                    symbol_kind: if kind.is_empty() {
                        None
                    } else {
                        Some(SymbolKind::from_str(&kind))
                    },
                    symbol_path: None,
                    scopes: Vec::new(),
                });
            }
        }

        chunks
    }

    fn extract_refs_from_tree(
        root: Node<'_>,
        content: &str,
        query: &Query,
    ) -> Vec<SymbolReference> {
        let mut cursor = QueryCursor::new();
        let mut refs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, root, content.as_bytes());
        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let capture_name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = &content[node.byte_range()];
                let line = node.start_position().row as i32;

                if is_primitive_or_keyword(text) {
                    continue;
                }

                let kind = match capture_name {
                    "call" => "call",
                    "import" => "import",
                    "type_usage" => "type_usage",
                    _ => continue,
                };

                let key = (text.to_string(), line, kind);
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);

                refs.push(SymbolReference {
                    id: 0,
                    source_symbol_id: 0,
                    target_name: text.to_string(),
                    target_symbol_id: None,
                    kind: kind.to_string(),
                    line,
                });
            }
        }

        refs
    }
}

/// Check if a name is a primitive type or common keyword (skip these in references)
fn is_primitive_or_keyword(name: &str) -> bool {
    matches!(
        name,
        // Rust primitives
        "bool" | "char" | "str" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64"
        // Rust common types
        | "Self" | "String" | "Vec" | "Option" | "Result" | "Box" | "Rc" | "Arc"
        | "Ok" | "Err" | "Some" | "None" | "true" | "false"
        // TypeScript/JavaScript primitives
        | "string" | "number" | "boolean" | "void" | "null" | "undefined"
        | "any" | "never" | "unknown" | "object" | "Array" | "Promise"
        | "console" | "JSON" | "Math" | "Date" | "Error"
        // Go primitives
        | "int" | "int8" | "int16" | "int32" | "int64"
        | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "complex64" | "complex128"
        | "byte" | "rune" | "error" | "nil"
    )
}

impl Default for CodeParser {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // SupportedLanguage tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_supported_language_from_extension() {
        assert_eq!(
            SupportedLanguage::from_extension("rs"),
            Some(SupportedLanguage::Rust)
        );
        assert_eq!(
            SupportedLanguage::from_extension("ts"),
            Some(SupportedLanguage::TypeScript)
        );
        assert_eq!(
            SupportedLanguage::from_extension("tsx"),
            Some(SupportedLanguage::TypeScript)
        );
        assert_eq!(
            SupportedLanguage::from_extension("js"),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(
            SupportedLanguage::from_extension("jsx"),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(
            SupportedLanguage::from_extension("vue"),
            Some(SupportedLanguage::Vue)
        );
        assert_eq!(
            SupportedLanguage::from_extension("py"),
            Some(SupportedLanguage::Python)
        );
        assert_eq!(
            SupportedLanguage::from_extension("go"),
            Some(SupportedLanguage::Go)
        );
        assert_eq!(SupportedLanguage::from_extension("txt"), None);
        assert_eq!(SupportedLanguage::from_extension(""), None);
    }

    #[test]
    fn test_is_primitive_or_keyword() {
        // Rust primitives
        assert!(is_primitive_or_keyword("bool"));
        assert!(is_primitive_or_keyword("String"));
        assert!(is_primitive_or_keyword("Vec"));
        assert!(is_primitive_or_keyword("Option"));

        // TypeScript primitives
        assert!(is_primitive_or_keyword("string"));
        assert!(is_primitive_or_keyword("number"));
        assert!(is_primitive_or_keyword("Promise"));

        // Go primitives
        assert!(is_primitive_or_keyword("int"));
        assert!(is_primitive_or_keyword("error"));

        // Custom names should not match
        assert!(!is_primitive_or_keyword("MyStruct"));
        assert!(!is_primitive_or_keyword("process_data"));
        assert!(!is_primitive_or_keyword("UserService"));
    }

    // -------------------------------------------------------------------------
    // Rust parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_rust_function() {
        let mut parser = CodeParser::new();
        let code = r#"
fn hello_world() {
    println!("Hello!");
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello_world");
        assert_eq!(symbols[0].kind, crate::models::chunk::SymbolKind::Function);
    }

    #[test]
    fn test_parse_rust_struct() {
        let mut parser = CodeParser::new();
        let code = r#"
pub struct User {
    pub id: i64,
    pub name: String,
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "User");
        assert_eq!(symbols[0].kind, crate::models::chunk::SymbolKind::Struct);
    }

    #[test]
    fn test_parse_rust_enum() {
        let mut parser = CodeParser::new();
        let code = r#"
enum Status {
    Active,
    Inactive,
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Status");
        assert_eq!(symbols[0].kind, crate::models::chunk::SymbolKind::Enum);
    }

    #[test]
    fn test_parse_rust_impl() {
        let mut parser = CodeParser::new();
        let code = r#"
impl User {
    fn new(name: String) -> Self {
        Self { id: 0, name }
    }
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        // Should capture impl block and the function inside
        assert!(symbols
            .iter()
            .any(|s| s.name == "User" && s.kind == crate::models::chunk::SymbolKind::Impl));
        assert!(symbols
            .iter()
            .any(|s| s.name == "new" && s.kind == crate::models::chunk::SymbolKind::Function));
    }

    #[test]
    fn test_parse_rust_trait() {
        let mut parser = CodeParser::new();
        let code = r#"
trait Printable {
    fn print(&self);
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "Printable" && s.kind == crate::models::chunk::SymbolKind::Trait));
    }

    #[test]
    fn test_parse_rust_imports() {
        let mut parser = CodeParser::new();
        let code = r#"
use std::collections::HashMap;
use crate::models::User;
use super::utils::helper;
"#;
        let imports = parser.parse_imports(code, SupportedLanguage::Rust);

        // std:: should be filtered out
        assert!(imports.iter().all(|i| !i.path.starts_with("std::")));

        // crate:: should be present and marked as relative
        assert!(imports
            .iter()
            .any(|i| i.path.contains("crate") && i.is_relative));

        // super:: should be present and marked as relative
        assert!(imports
            .iter()
            .any(|i| i.path.contains("super") && i.is_relative));
    }

    #[test]
    fn test_parse_rust_references() {
        let mut parser = CodeParser::new();
        let code = r#"
fn process() {
    let user = User::new("test".to_string());
    user.save();
    println!("Done");
}
"#;
        let refs = parser
            .parse_references(code, SupportedLanguage::Rust)
            .unwrap();

        // Should find calls to User::new and user.save
        assert!(refs
            .iter()
            .any(|r| r.target_name == "new" && r.kind == "call"));
        assert!(refs
            .iter()
            .any(|r| r.target_name == "save" && r.kind == "call"));

        // println! is a macro call
        assert!(refs
            .iter()
            .any(|r| r.target_name == "println" && r.kind == "call"));
    }

    // -------------------------------------------------------------------------
    // TypeScript parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_typescript_function() {
        let mut parser = CodeParser::new();
        let code = r#"
function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::TypeScript)
            .unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "greet" && s.kind == crate::models::chunk::SymbolKind::Function));
    }

    #[test]
    fn test_parse_typescript_class() {
        let mut parser = CodeParser::new();
        let code = r#"
class UserService {
    constructor(private db: Database) {}
    
    async getUser(id: number): Promise<User> {
        return this.db.find(id);
    }
}
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::TypeScript)
            .unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "UserService" && s.kind == crate::models::chunk::SymbolKind::Class));
        assert!(symbols
            .iter()
            .any(|s| s.name == "getUser" && s.kind == crate::models::chunk::SymbolKind::Method));
    }

    #[test]
    fn test_parse_typescript_interface() {
        let mut parser = CodeParser::new();
        let code = r#"
interface User {
    id: number;
    name: string;
    email?: string;
}
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::TypeScript)
            .unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "User" && s.kind == crate::models::chunk::SymbolKind::Interface));
    }

    #[test]
    fn test_parse_typescript_arrow_function() {
        let mut parser = CodeParser::new();
        let code = r#"
const processUser = (user: User): void => {
    console.log(user.name);
};
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::TypeScript)
            .unwrap();
        assert!(symbols.iter().any(|s| s.name == "processUser"));
    }

    #[test]
    fn test_parse_typescript_imports() {
        let mut parser = CodeParser::new();
        let code = r#"
import { User, Role } from './models';
import axios from 'axios';
import * as utils from '../utils';
"#;
        let imports = parser.parse_imports(code, SupportedLanguage::TypeScript);

        // Relative import
        assert!(imports
            .iter()
            .any(|i| i.path == "./models" && i.is_relative));

        // Package import
        assert!(imports.iter().any(|i| i.path == "axios" && !i.is_relative));

        // Named imports
        let models_import = imports.iter().find(|i| i.path == "./models").unwrap();
        assert!(models_import.items.contains(&"User".to_string()));
        assert!(models_import.items.contains(&"Role".to_string()));
    }

    #[test]
    fn test_parse_typescript_references() {
        let mut parser = CodeParser::new();
        let code = r#"
function test() {
    const user = new User();
    user.save();
    processData(user);
}
"#;
        let refs = parser
            .parse_references(code, SupportedLanguage::TypeScript)
            .unwrap();

        assert!(refs
            .iter()
            .any(|r| r.target_name == "User" && r.kind == "call"));
        assert!(refs
            .iter()
            .any(|r| r.target_name == "save" && r.kind == "call"));
        assert!(refs
            .iter()
            .any(|r| r.target_name == "processData" && r.kind == "call"));
    }

    // -------------------------------------------------------------------------
    // Python parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_python_function() {
        let mut parser = CodeParser::new();
        let code = r#"
def hello_world():
    print("Hello!")
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::Python)
            .unwrap();
        assert!(symbols.iter().any(
            |s| s.name == "hello_world" && s.kind == crate::models::chunk::SymbolKind::Function
        ));
    }

    #[test]
    fn test_parse_python_class() {
        let mut parser = CodeParser::new();
        let code = r#"
class UserService:
    def __init__(self, db):
        self.db = db
    
    def get_user(self, user_id: int) -> User:
        return self.db.find(user_id)
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::Python)
            .unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "UserService" && s.kind == crate::models::chunk::SymbolKind::Class));
        assert!(symbols.iter().any(|s| s.name == "__init__"));
        assert!(symbols.iter().any(|s| s.name == "get_user"));
    }

    #[test]
    fn test_parse_python_decorated_function() {
        let mut parser = CodeParser::new();
        let code = r#"
@app.route('/users')
def get_users():
    return jsonify(users)
"#;
        let symbols = parser
            .parse_symbols(code, SupportedLanguage::Python)
            .unwrap();
        assert!(symbols.iter().any(|s| s.name == "get_users"));
    }

    #[test]
    fn test_parse_python_imports() {
        let mut parser = CodeParser::new();
        let code = r#"
import os
from typing import List, Optional
from .models import User
from ..utils import helper
"#;
        let imports = parser.parse_imports(code, SupportedLanguage::Python);

        // Absolute import
        assert!(imports.iter().any(|i| i.path == "os" && !i.is_relative));

        // From import
        assert!(imports.iter().any(|i| i.path == "typing"));

        // Relative imports
        assert!(imports
            .iter()
            .any(|i| i.path.starts_with('.') && i.is_relative));
    }

    #[test]
    fn test_parse_python_references() {
        let mut parser = CodeParser::new();
        let code = r#"
def process():
    user = User.create("test")
    user.save()
    print("Done")
"#;
        let refs = parser
            .parse_references(code, SupportedLanguage::Python)
            .unwrap();

        assert!(refs
            .iter()
            .any(|r| r.target_name == "create" && r.kind == "call"));
        assert!(refs
            .iter()
            .any(|r| r.target_name == "save" && r.kind == "call"));
    }

    // -------------------------------------------------------------------------
    // Go parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_go_function() {
        let mut parser = CodeParser::new();
        let code = r#"
package main

func HelloWorld() {
    fmt.Println("Hello!")
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Go).unwrap();
        assert!(symbols.iter().any(
            |s| s.name == "HelloWorld" && s.kind == crate::models::chunk::SymbolKind::Function
        ));
    }

    #[test]
    fn test_parse_go_struct() {
        let mut parser = CodeParser::new();
        let code = r#"
package models

type User struct {
    ID   int64
    Name string
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Go).unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "User" && s.kind == crate::models::chunk::SymbolKind::Struct));
    }

    #[test]
    fn test_parse_go_interface() {
        let mut parser = CodeParser::new();
        let code = r#"
package service

type UserRepository interface {
    FindByID(id int64) (*User, error)
    Save(user *User) error
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Go).unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "UserRepository"
                && s.kind == crate::models::chunk::SymbolKind::Interface));
    }

    #[test]
    fn test_parse_go_method() {
        let mut parser = CodeParser::new();
        let code = r#"
package models

func (u *User) FullName() string {
    return u.FirstName + " " + u.LastName
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Go).unwrap();
        assert!(symbols
            .iter()
            .any(|s| s.name == "FullName" && s.kind == crate::models::chunk::SymbolKind::Method));
    }

    #[test]
    fn test_parse_go_imports() {
        let mut parser = CodeParser::new();
        let code = r#"
package main

import (
    "fmt"
    "github.com/gin-gonic/gin"
    "./internal/models"
)
"#;
        let imports = parser.parse_imports(code, SupportedLanguage::Go);

        assert!(imports.iter().any(|i| i.path == "fmt"));
        assert!(imports.iter().any(|i| i.path == "github.com/gin-gonic/gin"));
    }

    // -------------------------------------------------------------------------
    // Vue parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_vue_script() {
        let mut parser = CodeParser::new();
        let code = r#"
<template>
  <div>{{ message }}</div>
</template>

<script>
export default {
  name: 'HelloWorld',
  data() {
    return { message: 'Hello!' }
  }
}
</script>
"#;
        // Vue parsing extracts script and parses as TypeScript
        let symbols = parser.parse_symbols(code, SupportedLanguage::Vue).unwrap();
        // Should find the data method
        assert!(symbols.iter().any(|s| s.name == "data"));
    }

    #[test]
    fn test_parse_vue_script_setup() {
        let mut parser = CodeParser::new();
        let code = r#"
<template>
  <div>{{ count }}</div>
</template>

<script setup lang="ts">
import { ref } from 'vue'

const count = ref(0)

function increment() {
  count.value++
}
</script>
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Vue).unwrap();
        assert!(symbols.iter().any(|s| s.name == "increment"));
    }

    #[test]
    fn test_parse_vue_template_refs() {
        let mut parser = CodeParser::new();
        let code = r#"
<template>
  <UserCard :user="user" />
  <my-button @click="onClick">Click</my-button>
</template>

<script>
export default {
  components: { UserCard }
}
</script>
"#;
        let refs = parser
            .parse_references(code, SupportedLanguage::Vue)
            .unwrap();

        // Should detect component usage in template
        assert!(refs
            .iter()
            .any(|r| r.target_name == "UserCard" && r.kind == "component_usage"));
        assert!(refs
            .iter()
            .any(|r| r.target_name == "my-button" && r.kind == "component_usage"));
    }

    // -------------------------------------------------------------------------
    // parse_file (single-pass) tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_file_rust() {
        let mut parser = CodeParser::new();
        let code = r#"
use std::collections::HashMap;
use crate::models::User;

pub struct UserService {
    users: HashMap<i64, User>,
}

impl UserService {
    pub fn new() -> Self {
        Self { users: HashMap::new() }
    }
    
    pub fn get(&self, id: i64) -> Option<&User> {
        self.users.get(&id)
    }
}
"#;
        let result = parser
            .parse_file(code, "src/service.rs", SupportedLanguage::Rust)
            .unwrap();

        // Symbols
        assert!(result.symbols.iter().any(|s| s.name == "UserService"));
        assert!(result.symbols.iter().any(|s| s.name == "new"));
        assert!(result.symbols.iter().any(|s| s.name == "get"));

        // Imports (crate:: should be present)
        assert!(result.imports.iter().any(|i| i.path.contains("crate")));

        // Chunks should be non-empty
        assert!(!result.chunks.is_empty());
    }

    #[test]
    fn test_parse_file_typescript() {
        let mut parser = CodeParser::new();
        let code = r#"
import { User } from './models';
import axios from 'axios';

interface UserService {
    getUser(id: number): Promise<User>;
}

class UserServiceImpl implements UserService {
    async getUser(id: number): Promise<User> {
        const response = await axios.get(`/users/${id}`);
        return response.data;
    }
}

export const userService = new UserServiceImpl();
"#;
        let result = parser
            .parse_file(code, "src/service.ts", SupportedLanguage::TypeScript)
            .unwrap();

        // Symbols
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "UserService"
                && s.kind == crate::models::chunk::SymbolKind::Interface));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "UserServiceImpl"
                && s.kind == crate::models::chunk::SymbolKind::Class));
        assert!(result.symbols.iter().any(|s| s.name == "getUser"));

        // Imports
        assert!(result.imports.iter().any(|i| i.path == "./models"));
        assert!(result.imports.iter().any(|i| i.path == "axios"));

        // References
        assert!(result.refs.iter().any(|r| r.target_name == "get"));
    }

    // -------------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_empty_file() {
        let mut parser = CodeParser::new();
        let symbols = parser.parse_symbols("", SupportedLanguage::Rust).unwrap();
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_parse_comments_only() {
        let mut parser = CodeParser::new();
        let code = r#"
// This is a comment
/* Multi-line
   comment */
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_parse_syntax_error() {
        let mut parser = CodeParser::new();
        // Invalid Rust syntax - parser should handle gracefully
        let code = r#"
fn broken( {
    not valid rust
}
"#;
        // Should not panic, may return partial results or empty
        let result = parser.parse_symbols(code, SupportedLanguage::Rust);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_unicode() {
        let mut parser = CodeParser::new();
        let code = r#"
fn привет_мир() {
    println!("Привет, мир!");
}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();
        assert!(symbols.iter().any(|s| s.name == "привет_мир"));
    }

    #[test]
    fn test_parse_large_file() {
        let mut parser = CodeParser::new();
        // Generate a file with many functions
        let mut code = String::new();
        for i in 0..100 {
            code.push_str(&format!("fn func_{}() {{}}\n", i));
        }

        let symbols = parser
            .parse_symbols(&code, SupportedLanguage::Rust)
            .unwrap();
        assert_eq!(symbols.len(), 100);
    }

    #[test]
    fn test_line_numbers_correct() {
        let mut parser = CodeParser::new();
        let code = r#"// Line 0
// Line 1
fn first() {}

fn second() {}
"#;
        let symbols = parser.parse_symbols(code, SupportedLanguage::Rust).unwrap();

        let first = symbols.iter().find(|s| s.name == "first").unwrap();
        let second = symbols.iter().find(|s| s.name == "second").unwrap();

        // Line numbers are 0-indexed in tree-sitter
        assert_eq!(first.line_start, 2);
        assert_eq!(second.line_start, 4);
    }
}
