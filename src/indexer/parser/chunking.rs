use regex::Regex;
use tree_sitter::{Node, Parser};

use super::core::{ParserError, Result, SupportedLanguage};
use crate::models::{CodeChunk, SymbolKind};

// === Семантический AST-чанкинг (Smart Chunking) ===

/// Максимальный размер чанка в байтах (~2048 токенов)
const MAX_CHUNK_BYTES: usize = 8192;
/// Минимальный размер чанка (не создаём слишком мелкие)
const MIN_CHUNK_BYTES: usize = 256;

/// Семантический чанкинг файла на основе tree-sitter AST.
/// Уважает границы функций, классов и структур.
/// Oversized-узлы разбиваются на под-чанки с контекст-инъекцией (хлебные крошки).
pub fn smart_chunk_file(
    code: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Result<Vec<CodeChunk>> {
    // Vue — извлекаем <script>, чанкуем как TS
    if language == SupportedLanguage::Vue {
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
        SupportedLanguage::TypeScript,
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
    let mut parser = Parser::new();
    parser
        .set_language(&language.tree_sitter_language())
        .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

    let tree = parser.parse(code, None).ok_or(ParserError::ParseError)?;
    smart_chunk_from_root(tree.root_node(), code, file_path, language)
}

/// Smart chunking from an already-parsed tree root (avoids redundant parse).
pub(crate) fn smart_chunk_from_root(
    root: Node<'_>,
    code: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Result<Vec<CodeChunk>> {
    let mut chunks = Vec::new();
    let mut accumulator = ChunkAccumulator::new(file_path, code);

    // Обходим top-level узлы
    for i in 0..root.child_count() {
        let Some(child) = root.child(i as u32) else { continue };

        if is_significant_node(child.kind(), language) {
            let node_size = child.end_byte() - child.start_byte();

            if node_size > MAX_CHUNK_BYTES {
                // Сбрасываем накопленное
                accumulator.flush(&mut chunks);
                // Разбиваем oversized-узел
                chunk_oversized_node(child, code, file_path, language, &[], &mut chunks);
            } else if accumulator.size + node_size > MAX_CHUNK_BYTES
                && accumulator.size >= MIN_CHUNK_BYTES
            {
                // Не влезает — сбрасываем буфер и начинаем новый
                accumulator.flush(&mut chunks);
                accumulator.push_node(child, code, language);
            } else {
                // Накапливаем
                accumulator.push_node(child, code, language);
            }
        } else {
            // Незначимые узлы (комментарии, пустые строки) — прикрепляем к текущему буферу
            accumulator.push_raw(child, code);
        }
    }

    // Сбрасываем остаток
    accumulator.flush(&mut chunks);

    Ok(chunks)
}

/// Аккумулятор для склейки мелких AST-узлов в один чанк
struct ChunkAccumulator<'a> {
    file_path: &'a str,
    code: &'a str,
    /// Накопленные байтовые диапазоны
    ranges: Vec<(usize, usize)>,
    /// Текущий размер в байтах
    size: usize,
    /// Имя символа первого значимого узла
    first_symbol_name: Option<String>,
    /// Тип символа первого значимого узла
    first_symbol_kind: Option<SymbolKind>,
    /// symbol_path первого значимого узла
    first_symbol_path: Option<String>,
}

impl<'a> ChunkAccumulator<'a> {
    fn new(file_path: &'a str, code: &'a str) -> Self {
        Self {
            file_path,
            code,
            ranges: Vec::new(),
            size: 0,
            first_symbol_name: None,
            first_symbol_kind: None,
            first_symbol_path: None,
        }
    }

    fn push_node(&mut self, node: Node<'_>, code: &str, language: SupportedLanguage) {
        let start = node.start_byte();
        let end = node.end_byte();
        let text = &code[start..end];

        if self.first_symbol_name.is_none() {
            let (name, kind, path) = extract_node_meta(node, code, language);
            if name.is_some() {
                self.first_symbol_name = name;
                self.first_symbol_kind = kind;
                self.first_symbol_path = path;
            }
        }

        self.ranges.push((start, end));
        self.size += text.len();
    }

    fn push_raw(&mut self, node: Node<'_>, _code: &str) {
        let start = node.start_byte();
        let end = node.end_byte();
        self.ranges.push((start, end));
        self.size += end - start;
    }

    fn flush(&mut self, chunks: &mut Vec<CodeChunk>) {
        if self.ranges.is_empty() || self.size < MIN_CHUNK_BYTES {
            if !self.ranges.is_empty() {
                // Слишком маленький — не теряем, приклеим к следующему
                return;
            }
            return;
        }

        // Собираем контент из сохранённых диапазонов с промежутками
        // Safety: ranges is guaranteed non-empty by the early return above
        let (min_start, _) = match self.ranges.first() {
            Some(r) => *r,
            None => return, // Should never happen, but safe fallback
        };
        let (_, max_end) = match self.ranges.last() {
            Some(r) => *r,
            None => return,
        };
        let content = self.code[min_start..max_end].to_string();

        // Линии
        let line_start = self.code[..min_start].lines().count() as u32;
        let line_end = self.code[..max_end].lines().count() as u32;

        let id = format!("{}:{}:{}", self.file_path, line_start, line_end);

        chunks.push(CodeChunk {
            id,
            file_path: self.file_path.to_string(),
            content,
            line_start,
            line_end,
            symbol_name: self.first_symbol_name.take(),
            symbol_kind: self.first_symbol_kind.take(),
            symbol_path: self.first_symbol_path.take(),
            scopes: Vec::new(),
        });

        self.ranges.clear();
        self.size = 0;
    }
}

/// Разбиение oversized-узла (большая функция, огромный impl) на под-чанки
fn chunk_oversized_node(
    node: Node<'_>,
    code: &str,
    file_path: &str,
    language: SupportedLanguage,
    parent_scopes: &[String],
    chunks: &mut Vec<CodeChunk>,
) {
    let kind = node.kind();

    // Строим стек скоупов
    let mut scopes = parent_scopes.to_vec();
    let (name, sym_kind, _) = extract_node_meta(node, code, language);
    if let Some(ref n) = name {
        let label = if let Some(ref k) = sym_kind {
            format!("{} {}", k, n)
        } else {
            n.clone()
        };
        scopes.push(label);
    }

    // Если это контейнер (impl, class, module) — пробуем рекурсивно разбить по дочерним
    if is_container_node(kind, language) {
        let mut acc = ChunkAccumulator::new(file_path, code);

        for i in 0..node.child_count() {
            let Some(child) = node.child(i as u32) else { continue };
            let child_size = child.end_byte() - child.start_byte();

            if child_size > MAX_CHUNK_BYTES {
                acc.flush(chunks);
                // Рекурсия для вложенных oversized (метод внутри impl)
                chunk_oversized_node(child, code, file_path, language, &scopes, chunks);
            } else if is_significant_node(child.kind(), language) {
                if acc.size + child_size > MAX_CHUNK_BYTES && acc.size >= MIN_CHUNK_BYTES {
                    acc.flush(chunks);
                }
                acc.push_node(child, code, language);
            } else {
                acc.push_raw(child, code);
            }
        }

        // Прописываем scopes в созданных чанках
        acc.flush(chunks);
        let chunk_count = chunks.len();
        for chunk in chunks
            .iter_mut()
            .skip(chunk_count.saturating_sub(node.child_count()))
        {
            if chunk.scopes.is_empty() {
                chunk.scopes = scopes.clone();
            }
        }
        return;
    }

    // Для атомарных oversized-узлов (гигантская функция) — разрезаем по строкам с context injection
    let node_text = &code[node.start_byte()..node.end_byte()];
    let node_line_start = code[..node.start_byte()].lines().count() as u32;

    let context_prefix = if !scopes.is_empty() {
        format!("// Context: {}\n", scopes.join(" -> "))
    } else if let Some(ref n) = name {
        let k = sym_kind.as_ref().map(|s| s.as_str()).unwrap_or("fn");
        format!("// Context: {} {}\n", k, n)
    } else {
        String::new()
    };

    let prefix_len = context_prefix.len();
    let effective_max = MAX_CHUNK_BYTES.saturating_sub(prefix_len);

    let mut current_start = 0usize;
    let mut current_size = 0usize;
    let mut chunk_line_start = node_line_start;
    let lines: Vec<&str> = node_text.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let line_size = line.len() + 1; // +1 для \n

        if current_size + line_size > effective_max && current_size > 0 {
            // Сбрасываем чанк
            let chunk_text = &node_text[current_start..current_start + current_size];
            let content = if !context_prefix.is_empty() {
                format!("{}{}", context_prefix, chunk_text)
            } else {
                chunk_text.to_string()
            };

            let line_end = chunk_line_start
                + (node_text[current_start..current_start + current_size]
                    .lines()
                    .count() as u32);

            chunks.push(CodeChunk {
                id: format!("{}:{}:{}", file_path, chunk_line_start, line_end),
                file_path: file_path.to_string(),
                content,
                line_start: chunk_line_start,
                line_end,
                symbol_name: name.clone(),
                symbol_kind: sym_kind,
                symbol_path: None,
                scopes: scopes.clone(),
            });

            chunk_line_start = node_line_start + i as u32;
            current_start += current_size;
            current_size = 0;
        }

        current_size += line_size;
    }

    // Последний под-чанк
    if current_size > 0 {
        let chunk_text = &node_text[current_start..];
        let content = if !context_prefix.is_empty() && current_start > 0 {
            format!("{}{}", context_prefix, chunk_text)
        } else {
            chunk_text.to_string()
        };

        let line_end = node_line_start + lines.len() as u32;

        chunks.push(CodeChunk {
            id: format!("{}:{}:{}", file_path, chunk_line_start, line_end),
            file_path: file_path.to_string(),
            content,
            line_start: chunk_line_start,
            line_end,
            symbol_name: name,
            symbol_kind: sym_kind,
            symbol_path: None,
            scopes,
        });
    }
}

/// Определяет, является ли узел значимой единицей кода (функция, struct, class, impl, etc.)
fn is_significant_node(kind: &str, language: SupportedLanguage) -> bool {
    match language {
        SupportedLanguage::Rust => matches!(
            kind,
            "function_item"
                | "struct_item"
                | "enum_item"
                | "impl_item"
                | "trait_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "macro_definition"
                | "macro_invocation"
                | "use_declaration"
                | "mod_item"
                | "attribute_item"
        ),
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "lexical_declaration"
                | "variable_declaration"
                | "export_statement"
                | "import_statement"
                | "method_definition"
                | "abstract_class_declaration"
        ),
        SupportedLanguage::Python => matches!(
            kind,
            "function_definition"
                | "class_definition"
                | "decorated_definition"
                | "import_statement"
                | "import_from_statement"
                | "assignment"
                | "expression_statement"
        ),
        SupportedLanguage::Vue => false, // Vue обрабатывается через извлечение <script>
        SupportedLanguage::Go => matches!(
            kind,
            "function_declaration"
                | "method_declaration"
                | "type_declaration"
                | "const_declaration"
                | "var_declaration"
                | "import_declaration"
        ),
    }
}

/// Определяет, является ли узел контейнером, который можно разбить по дочерним
fn is_container_node(kind: &str, language: SupportedLanguage) -> bool {
    match language {
        SupportedLanguage::Rust => matches!(
            kind,
            "impl_item" | "trait_item" | "mod_item" | "declaration_list"
        ),
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => matches!(
            kind,
            "class_declaration" | "class_body" | "abstract_class_declaration"
        ),
        SupportedLanguage::Python => matches!(kind, "class_definition"),
        SupportedLanguage::Vue => false,
        SupportedLanguage::Go => matches!(kind, "type_declaration"),
    }
}

/// Извлекает метаданные узла: (name, kind, symbol_path)
fn extract_node_meta(
    node: Node<'_>,
    code: &str,
    language: SupportedLanguage,
) -> (Option<String>, Option<SymbolKind>, Option<String>) {
    let kind_str = node.kind();

    // Определяем человекочитаемый тип
    let sym_kind = match kind_str {
        "function_item" | "function_declaration" | "function_definition" => {
            Some(SymbolKind::Function)
        }
        "struct_item" => Some(SymbolKind::Struct),
        "enum_item" | "enum_declaration" => Some(SymbolKind::Enum),
        "impl_item" => Some(SymbolKind::Impl),
        "trait_item" => Some(SymbolKind::Trait),
        "class_declaration" | "class_definition" | "abstract_class_declaration" => {
            Some(SymbolKind::Struct)
        } // Classes as structs
        "interface_declaration" => Some(SymbolKind::Trait), // Interfaces as traits
        "type_alias_declaration" | "type_item" => Some(SymbolKind::Type),
        "const_item" | "lexical_declaration" | "const_declaration" => Some(SymbolKind::Const),
        "method_definition" | "method_declaration" => Some(SymbolKind::Function), // Methods as functions
        "var_declaration" => Some(SymbolKind::Const),                             // Var as const
        "type_declaration" => {
            // Go type_declaration — look inside for struct/interface
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    if child.kind() == "type_spec" {
                        let name = child
                            .child_by_field_name("name")
                            .map(|n| code[n.byte_range()].to_string());
                        let inner_type = child.child_by_field_name("type");
                        let kind = inner_type
                            .map(|t| match t.kind() {
                                "struct_type" => SymbolKind::Struct,
                                "interface_type" => SymbolKind::Trait,
                                _ => SymbolKind::Type,
                            })
                            .unwrap_or(SymbolKind::Type);
                        return (name, Some(kind), None);
                    }
                }
            }
            Some(SymbolKind::Type)
        }
        "decorated_definition" => {
            // Заглядываем внутрь: ищем function_definition или class_definition
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    if child.kind() == "function_definition" || child.kind() == "class_definition" {
                        return extract_node_meta(child, code, language);
                    }
                }
            }
            None
        }
        "export_statement" => {
            // Заглядываем внутрь exported declaration
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    if is_significant_node(child.kind(), language) {
                        return extract_node_meta(child, code, language);
                    }
                }
            }
            Some(SymbolKind::Module) // Export as module-level
        }
        _ => None,
    };

    // Извлекаем имя из поля "name"
    let name = node
        .child_by_field_name("name")
        .map(|n| code[n.byte_range()].to_string());

    // symbol_path — для простых случаев совпадает с name
    let symbol_path = name.clone();

    (name, sym_kind, symbol_path)
}
