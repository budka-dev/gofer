use regex::Regex;
use tree_sitter::Node;

use super::core::{ParserError, Result, SupportedLanguage};
use crate::models::{CodeChunk, SymbolKind};

// === Семантический AST-чанкинг (Smart Chunking) ===

/// Максимальный размер чанка в байтах (~2048 токенов)
const MAX_CHUNK_BYTES: usize = 8192;
/// Минимальный размер чанка (не создаём слишком мелкие)
const MIN_CHUNK_BYTES: usize = 256;

/// Семантический чанкинг файла на основе tree-sitter AST.
pub fn smart_chunk_file(
    code: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Result<Vec<CodeChunk>> {
    if language.name() == SupportedLanguage::VUE {
        return smart_chunk_vue(code, file_path);
    }

    smart_chunk_internal(code, file_path, language)
}

fn smart_chunk_vue(code: &str, file_path: &str) -> Result<Vec<CodeChunk>> {
    let script_re = Regex::new(r"(?s)<script[^>]*>\n?(.*?)</script>").unwrap();

    let Some(caps) = script_re.captures(code) else {
        return Ok(Vec::new());
    };

    let script_body = caps.get(1).unwrap();
    let line_offset = code[..script_body.start()].lines().count() as u32;

    let mut chunks = smart_chunk_internal(
        script_body.as_str(),
        file_path,
        SupportedLanguage(SupportedLanguage::TYPESCRIPT.to_string()),
    )?;

    for chunk in &mut chunks {
        chunk.line_start += line_offset;
        chunk.line_end += line_offset;
        chunk.id = format!("{}:{}:{}", file_path, chunk.line_start, chunk.line_end);
    }

    Ok(chunks)
}

fn smart_chunk_internal(
    code: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Result<Vec<CodeChunk>> {
    let lang = crate::indexer::parser::LANG_MANAGER.get_language(language.name())
        .ok_or_else(|| ParserError::UnsupportedLanguage(language.name().to_string()))?;
    
    let tree = crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&lang.language).map_err(|e| ParserError::LangManagerError(e.to_string()))?;
        parser.parse(code, None).ok_or(ParserError::ParseError)
    })?;
    smart_chunk_from_root(tree.root_node(), code, file_path, language)
}

pub(crate) fn smart_chunk_from_root(
    root: Node<'_>,
    code: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Result<Vec<CodeChunk>> {
    let mut chunks = Vec::new();
    let mut accumulator = ChunkAccumulator::new(file_path, code);

    for i in 0..root.child_count() {
        let Some(child) = root.child(i as u32) else { continue };

        if is_significant_node(child.kind(), &language) {
            let node_size = child.end_byte() - child.start_byte();

            if node_size > MAX_CHUNK_BYTES {
                accumulator.flush(&mut chunks);
                chunk_oversized_node(child, code, file_path, &language, &[], &mut chunks);
            } else if accumulator.size + node_size > MAX_CHUNK_BYTES
                && accumulator.size >= MIN_CHUNK_BYTES
            {
                accumulator.flush(&mut chunks);
                accumulator.push_node(child, code, &language);
            } else {
                accumulator.push_node(child, code, &language);
            }
        } else {
            accumulator.push_node(child, code, &language);
        }
    }

    accumulator.flush(&mut chunks);
    Ok(chunks)
}

struct ChunkAccumulator<'a> {
    file_path: &'a str,
    code: &'a str,
    nodes: Vec<Node<'a>>,
    size: usize,
}

impl<'a> ChunkAccumulator<'a> {
    fn new(file_path: &'a str, code: &'a str) -> Self {
        Self {
            file_path,
            code,
            nodes: Vec::new(),
            size: 0,
        }
    }

    fn push_node(&mut self, node: Node<'a>, _code: &str, _language: &SupportedLanguage) {
        self.size += node.end_byte() - node.start_byte();
        self.nodes.push(node);
    }

    fn flush(&mut self, chunks: &mut Vec<CodeChunk>) {
        if self.nodes.is_empty() {
            return;
        }

        let start_byte = self.nodes.first().unwrap().start_byte();
        let end_byte = self.nodes.last().unwrap().end_byte();
        let content = self.code[start_byte..end_byte].to_string();

        let line_start = self.nodes.first().unwrap().start_position().row as u32;
        let line_end = self.nodes.last().unwrap().end_position().row as u32;

        chunks.push(CodeChunk {
            id: format!("{}:{}:{}", self.file_path, line_start, line_end),
            file_path: self.file_path.to_string(),
            content,
            line_start,
            line_end,
            symbol_name: None,
            symbol_kind: None,
            symbol_path: None,
            scopes: Vec::new(),
        });

        self.nodes.clear();
        self.size = 0;
    }
}

fn chunk_oversized_node<'a>(
    node: Node<'a>,
    code: &'a str,
    file_path: &'a str,
    language: &SupportedLanguage,
    scopes: &[String],
    chunks: &mut Vec<CodeChunk>,
) {
    let kind = node.kind();
    let (name, sym_kind, _) = extract_node_meta(node, code, language);
    
    let mut current_scopes = scopes.to_vec();
    if let Some(n) = name.clone() {
        current_scopes.push(n);
    }

    if is_container_node(kind, language) {
        let mut acc = ChunkAccumulator::new(file_path, code);
        for i in 0..node.child_count() {
            let Some(child) = node.child(i as u32) else { continue };
            let child_size = child.end_byte() - child.start_byte();

            if child_size > MAX_CHUNK_BYTES {
                acc.flush(chunks);
                chunk_oversized_node(child, code, file_path, language, &current_scopes, chunks);
            } else if is_significant_node(child.kind(), language) {
                acc.flush(chunks);
                acc.push_node(child, code, language);
            } else {
                acc.push_node(child, code, language);
            }
        }
        acc.flush(chunks);
    } else {
        let lines: Vec<&str> = code[node.byte_range()].lines().collect();
        let mut current_chunk = Vec::new();
        let mut current_size = 0;
        let mut chunk_start_line = node.start_position().row as u32;

        for (i, line) in lines.iter().enumerate() {
            if current_size + line.len() > MAX_CHUNK_BYTES && !current_chunk.is_empty() {
                let content = current_chunk.join("\n");
                let line_end = chunk_start_line + i as u32;
                
                chunks.push(CodeChunk {
                    id: format!("{}:{}:{}", file_path, chunk_start_line, line_end),
                    file_path: file_path.to_string(),
                    content,
                    line_start: chunk_start_line,
                    line_end,
                    symbol_name: name.clone(),
                    symbol_kind: sym_kind,
                    symbol_path: Some(current_scopes.join(".")),
                    scopes: current_scopes.clone(),
                });
                
                current_chunk.clear();
                current_size = 0;
                chunk_start_line = line_end;
            }
            current_chunk.push(line.to_string());
            current_size += line.len() + 1;
        }

        if !current_chunk.is_empty() {
            let content = current_chunk.join("\n");
            let line_end = node.end_position().row as u32;
            chunks.push(CodeChunk {
                id: format!("{}:{}:{}", file_path, chunk_start_line, line_end),
                file_path: file_path.to_string(),
                content,
                line_start: chunk_start_line,
                line_end,
                symbol_name: name,
                symbol_kind: sym_kind,
                symbol_path: Some(current_scopes.join(".")),
                scopes: current_scopes,
            });
        }
    }
}

fn is_significant_node(kind: &str, language: &SupportedLanguage) -> bool {
    match language.name() {
        SupportedLanguage::RUST => matches!(kind, "function_item" | "struct_item" | "enum_item" | "impl_item" | "trait_item"),
        SupportedLanguage::TYPESCRIPT | SupportedLanguage::JAVASCRIPT => matches!(kind, "function_declaration" | "class_declaration" | "interface_declaration"),
        SupportedLanguage::PYTHON => matches!(kind, "function_definition" | "class_definition"),
        SupportedLanguage::GO => matches!(kind, "function_declaration" | "type_declaration"),
        _ => false,
    }
}

fn is_container_node(kind: &str, language: &SupportedLanguage) -> bool {
    match language.name() {
        SupportedLanguage::RUST => matches!(kind, "impl_item" | "trait_item" | "mod_item"),
        SupportedLanguage::TYPESCRIPT | SupportedLanguage::JAVASCRIPT => matches!(kind, "class_declaration" | "class_body"),
        SupportedLanguage::PYTHON => matches!(kind, "class_definition"),
        SupportedLanguage::GO => matches!(kind, "type_declaration"),
        _ => false,
    }
}

fn extract_node_meta(
    node: Node<'_>,
    code: &str,
    _language: &SupportedLanguage,
) -> (Option<String>, Option<SymbolKind>, Option<String>) {
    let kind_str = node.kind();
    let sym_kind = match kind_str {
        "function_item" | "function_declaration" | "function_definition" => Some(SymbolKind::Function),
        "struct_item" | "class_declaration" | "class_definition" => Some(SymbolKind::Struct),
        "enum_item" | "enum_declaration" => Some(SymbolKind::Enum),
        _ => None,
    };

    let name = node.child_by_field_name("name").map(|n| code[n.byte_range()].to_string());
    (name.clone(), sym_kind, name)
}
