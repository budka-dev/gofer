use regex::Regex;
use tree_sitter::{Node, Parser};

use super::core::{ParserError, Result, SupportedLanguage};

// === Code Skeletonization ===

/// Генерирует скелет кода: удаляет тела функций/методов, сохраняя сигнатуры,
/// типы, импорты, doc-комментарии и атрибуты.
/// Тела заменяются на `{ /* ... */ }` (или `...` для Python).
pub fn generate_skeleton(code: &str, language: SupportedLanguage) -> Result<String> {
    // Для Vue: извлекаем <script>, скелетизируем как TS, собираем обратно
    if language == SupportedLanguage::Vue {
        return generate_vue_skeleton(code);
    }

    generate_skeleton_internal(code, language)
}

fn generate_skeleton_internal(code: &str, language: SupportedLanguage) -> Result<String> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.tree_sitter_language())
        .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

    let tree = parser.parse(code, None).ok_or(ParserError::ParseError)?;
    let root = tree.root_node();

    // Собираем байтовые диапазоны тел, которые нужно заменить
    let mut replacements: Vec<(usize, usize, &str)> = Vec::new();

    collect_body_ranges(root, code, language, &mut replacements);

    // Сортируем по позиции (от конца к началу — не нужно, склеиваем от начала)
    replacements.sort_by_key(|r| r.0);

    // Удаляем перекрывающиеся диапазоны (оставляем внешний)
    let merged = merge_ranges(&replacements);

    // Склеиваем результат
    let bytes = code.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;

    for (start, end, stub) in &merged {
        if *start > cursor {
            result.extend_from_slice(&bytes[cursor..*start]);
        }
        result.extend_from_slice(stub.as_bytes());
        cursor = *end;
    }

    if cursor < bytes.len() {
        result.extend_from_slice(&bytes[cursor..]);
    }

    String::from_utf8(result).map_err(|_| ParserError::ParseError)
}

/// Рекурсивный обход AST для сбора диапазонов тел функций
fn collect_body_ranges(
    node: Node<'_>,
    code: &str,
    language: SupportedLanguage,
    replacements: &mut Vec<(usize, usize, &str)>,
) {
    match language {
        SupportedLanguage::Rust => collect_rust_bodies(node, code, replacements),
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
            collect_ts_bodies(node, code, replacements)
        }
        SupportedLanguage::Python => collect_python_bodies(node, code, replacements),
        SupportedLanguage::Vue => {} // обрабатывается отдельно
        SupportedLanguage::Go => collect_go_bodies(node, code, replacements),
    }
}

/// Rust: заменяем block-тела function_item и fn внутри impl
fn collect_rust_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();

    match kind {
        "function_item" => {
            // Ищем дочерний узел типа "block" — это тело функции
            if let Some(body) = node.child_by_field_name("body") {
                if body.kind() == "block" {
                    replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                    return; // не рекурсируем внутрь тела
                }
            }
        }
        // impl_item — рекурсируем внутрь, чтобы найти вложенные function_item
        "impl_item" | "trait_item" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    collect_rust_bodies(child, _code, replacements);
                }
            }
            return;
        }
        // declaration_list внутри impl/trait
        "declaration_list" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    collect_rust_bodies(child, _code, replacements);
                }
            }
            return;
        }
        _ => {}
    }

    // Рекурсия для верхнеуровневых узлов (mod, source_file)
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_rust_bodies(child, _code, replacements);
        }
    }
}

/// TypeScript/JavaScript: заменяем statement_block в function/method/arrow
fn collect_ts_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();

    match kind {
        "function_declaration" | "method_definition" | "function" => {
            if let Some(body) = node.child_by_field_name("body") {
                if body.kind() == "statement_block" {
                    replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                    return;
                }
            }
        }
        "arrow_function" => {
            if let Some(body) = node.child_by_field_name("body") {
                if body.kind() == "statement_block" {
                    replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                    return;
                }
                // Если тело — выражение (не блок), не трогаем (однострочная стрелка)
            }
        }
        // class_body — рекурсируем внутрь для методов
        "class_body" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    collect_ts_bodies(child, _code, replacements);
                }
            }
            return;
        }
        _ => {}
    }

    // Рекурсия для верхнеуровневых узлов
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_ts_bodies(child, _code, replacements);
        }
    }
}

/// Python: заменяем block-тело function_definition на `...`
fn collect_python_bodies(
    node: Node<'_>,
    _code: &str,
    replacements: &mut Vec<(usize, usize, &str)>,
) {
    let kind = node.kind();

    match kind {
        "function_definition" => {
            if let Some(body) = node.child_by_field_name("body") {
                if body.kind() == "block" {
                    replacements.push((body.start_byte(), body.end_byte(), "..."));
                    return;
                }
            }
        }
        // decorated_definition — рекурсируем внутрь
        "decorated_definition" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    collect_python_bodies(child, _code, replacements);
                }
            }
            return;
        }
        // class_definition — рекурсируем для методов
        "class_definition" => {
            if let Some(body) = node.child_by_field_name("body") {
                for i in 0..body.child_count() {
                    if let Some(child) = body.child(i as u32) {
                        collect_python_bodies(child, _code, replacements);
                    }
                }
            }
            return;
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_python_bodies(child, _code, replacements);
        }
    }
}

/// Vue: извлекаем <script>, скелетизируем как TypeScript, собираем обратно
fn generate_vue_skeleton(code: &str) -> Result<String> {
    let script_re = Regex::new(r"(?s)(<script[^>]*>)(.*?)(</script>)").unwrap();

    let Some(caps) = script_re.captures(code) else {
        // Нет <script> — возвращаем как есть
        return Ok(code.to_string());
    };

    let open_tag = caps.get(1).unwrap();
    let script_body = caps.get(2).unwrap();
    let close_tag = caps.get(3).unwrap();

    let skeleton_script =
        generate_skeleton_internal(script_body.as_str(), SupportedLanguage::TypeScript)?;

    let mut result = String::with_capacity(code.len());
    result.push_str(&code[..open_tag.end()]);
    result.push_str(&skeleton_script);
    result.push_str(&code[close_tag.start()..]);

    Ok(result)
}

/// Удаляем перекрывающиеся/вложенные диапазоны, оставляя только внешний
fn merge_ranges<'a>(ranges: &[(usize, usize, &'a str)]) -> Vec<(usize, usize, &'a str)> {
    let mut merged: Vec<(usize, usize, &str)> = Vec::new();

    for &(start, end, stub) in ranges {
        if let Some(last) = merged.last() {
            // Если текущий диапазон вложен в предыдущий — пропускаем
            if start >= last.0 && end <= last.1 {
                continue;
            }
        }
        merged.push((start, end, stub));
    }

    merged
}

// --- Go: struct fields ---

fn collect_go_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();

    match kind {
        "function_declaration" | "method_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                if body.kind() == "block" {
                    replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                    return;
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_go_bodies(child, _code, replacements);
        }
    }
}
