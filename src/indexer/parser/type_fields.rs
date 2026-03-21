use regex::Regex;
use tree_sitter::{Node, Parser};

use super::core::{ParserError, Result, SupportedLanguage};
use crate::models::TypeField;

// === Structural Fingerprinting (извлечение полей struct/interface через AST) ===

/// Нормализация имени поля: snake_case/camelCase -> lowercase без разделителей
/// "user_id" -> "userid", "userId" -> "userid", "UserID" -> "userid"
pub fn normalize_field(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        if c == '_' || c == '-' {
            continue;
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

/// Извлечь поля из struct/interface/type по имени через tree-sitter AST
#[allow(dead_code)]
pub fn parse_type_fields(
    code: &str,
    type_name: &str,
    language: SupportedLanguage,
) -> Result<Vec<TypeField>> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.tree_sitter_language())
        .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

    let tree = parser.parse(code, None).ok_or(ParserError::ParseError)?;
    let root = tree.root_node();

    match language {
        SupportedLanguage::Rust => extract_rust_struct_fields(root, code, type_name),
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
            extract_ts_interface_fields(root, code, type_name)
        }
        SupportedLanguage::Python => extract_python_class_fields(root, code, type_name),
        SupportedLanguage::Vue => {
            // Vue — извлекаем из <script>
            let script_re = Regex::new(r"(?s)<script[^>]*>\n?(.*?)</script>").unwrap();
            if let Some(caps) = script_re.captures(code) {
                let script_body = caps.get(1).unwrap().as_str();
                let mut p2 = Parser::new();
                p2.set_language(&SupportedLanguage::TypeScript.tree_sitter_language())
                    .map_err(|_| ParserError::UnsupportedLanguage("vue/ts".into()))?;
                let tree2 = p2.parse(script_body, None).ok_or(ParserError::ParseError)?;
                extract_ts_interface_fields(tree2.root_node(), script_body, type_name)
            } else {
                Ok(Vec::new())
            }
        }
        SupportedLanguage::Go => extract_go_struct_fields(root, code, type_name),
    }
}

/// Извлечь все типы с полями из файла (для batch-индексации)
pub fn parse_all_type_fields(
    code: &str,
    language: SupportedLanguage,
) -> Result<Vec<(String, Vec<TypeField>)>> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.tree_sitter_language())
        .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", language)))?;

    let actual_code = if language == SupportedLanguage::Vue {
        let script_re = Regex::new(r"(?s)<script[^>]*>\n?(.*?)</script>").unwrap();
        match script_re.captures(code) {
            Some(caps) => caps.get(1).unwrap().as_str().to_string(),
            None => return Ok(Vec::new()),
        }
    } else {
        code.to_string()
    };

    let ts_lang = if language == SupportedLanguage::Vue {
        SupportedLanguage::TypeScript
    } else {
        language
    };

    let mut p = Parser::new();
    p.set_language(&ts_lang.tree_sitter_language())
        .map_err(|_| ParserError::UnsupportedLanguage(format!("{:?}", ts_lang)))?;

    let tree = p.parse(&actual_code, None).ok_or(ParserError::ParseError)?;
    let root = tree.root_node();

    let mut results = Vec::new();

    match ts_lang {
        SupportedLanguage::Rust => collect_all_rust_structs(root, &actual_code, &mut results),
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
            collect_all_ts_interfaces(root, &actual_code, &mut results)
        }
        SupportedLanguage::Python => collect_all_python_classes(root, &actual_code, &mut results),
        SupportedLanguage::Go => collect_all_go_structs(root, &actual_code, &mut results),
        _ => {}
    }

    Ok(results)
}

// --- Rust: struct fields ---

fn extract_rust_struct_fields(
    root: Node<'_>,
    code: &str,
    type_name: &str,
) -> Result<Vec<TypeField>> {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        if node.kind() == "struct_item" {
            if let Some(name_node) = node.child_by_field_name("name") {
                if &code[name_node.byte_range()] == type_name {
                    return Ok(extract_fields_from_rust_struct(node, code));
                }
            }
        }
    }
    Ok(Vec::new())
}

fn collect_all_rust_structs(
    root: Node<'_>,
    code: &str,
    results: &mut Vec<(String, Vec<TypeField>)>,
) {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        if node.kind() == "struct_item" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = code[name_node.byte_range()].to_string();
                let fields = extract_fields_from_rust_struct(node, code);
                if !fields.is_empty() {
                    results.push((name, fields));
                }
            }
        }
    }
}

fn extract_fields_from_rust_struct(node: Node<'_>, code: &str) -> Vec<TypeField> {
    let mut fields = Vec::new();

    // Ищем field_declaration_list
    if let Some(body) = node.child_by_field_name("body") {
        for i in 0..body.child_count() {
            let Some(child) = body.child(i as u32) else { continue };
            if child.kind() == "field_declaration" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = code[name_node.byte_range()].to_string();
                    let field_type = child
                        .child_by_field_name("type")
                        .map(|t| code[t.byte_range()].to_string());
                    let normalized = normalize_field(&name);
                    fields.push(TypeField {
                        name,
                        field_type,
                        normalized,
                    });
                }
            }
        }
    }

    fields
}

// --- TypeScript/JavaScript: interface/type fields ---

fn extract_ts_interface_fields(
    root: Node<'_>,
    code: &str,
    type_name: &str,
) -> Result<Vec<TypeField>> {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        let kind = node.kind();

        // interface_declaration, type_alias_declaration
        if kind == "interface_declaration" || kind == "type_alias_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                if &code[name_node.byte_range()] == type_name {
                    return Ok(extract_fields_from_ts_node(node, code));
                }
            }
        }
        // export_statement может обёртывать
        if kind == "export_statement" {
            for j in 0..node.child_count() {
                let Some(inner) = node.child(j as u32) else { continue };
                if inner.kind() == "interface_declaration"
                    || inner.kind() == "type_alias_declaration"
                {
                    if let Some(name_node) = inner.child_by_field_name("name") {
                        if &code[name_node.byte_range()] == type_name {
                            return Ok(extract_fields_from_ts_node(inner, code));
                        }
                    }
                }
            }
        }
    }
    Ok(Vec::new())
}

fn collect_all_ts_interfaces(
    root: Node<'_>,
    code: &str,
    results: &mut Vec<(String, Vec<TypeField>)>,
) {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        collect_ts_interface_recursive(node, code, results);
    }
}

fn collect_ts_interface_recursive(
    node: Node<'_>,
    code: &str,
    results: &mut Vec<(String, Vec<TypeField>)>,
) {
    let kind = node.kind();

    if kind == "interface_declaration" || kind == "type_alias_declaration" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = code[name_node.byte_range()].to_string();
            let fields = extract_fields_from_ts_node(node, code);
            if !fields.is_empty() {
                results.push((name, fields));
            }
        }
        return;
    }

    // Рекурсия для export_statement и прочих обёрток
    if kind == "export_statement" || kind == "program" {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                collect_ts_interface_recursive(child, code, results);
            }
        }
    }
}

fn extract_fields_from_ts_node(node: Node<'_>, code: &str) -> Vec<TypeField> {
    let mut fields = Vec::new();

    // Ищем object_type или interface_body
    fn find_body<'a>(node: Node<'a>) -> Option<Node<'a>> {
        if node.kind() == "object_type" || node.kind() == "interface_body" {
            return Some(node);
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if let Some(found) = find_body(child) {
                    return Some(found);
                }
            }
        }
        None
    }

    let Some(body) = find_body(node) else {
        return fields;
    };

    for i in 0..body.child_count() {
        let Some(child) = body.child(i as u32) else { continue };
        // property_signature (interface) или property_type (object_type literal)
        if child.kind() == "property_signature" || child.kind() == "property_identifier" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = code[name_node.byte_range()].to_string();
                let field_type = child
                    .child_by_field_name("type")
                    .map(|t| code[t.byte_range()].to_string());
                let normalized = normalize_field(&name);
                fields.push(TypeField {
                    name,
                    field_type,
                    normalized,
                });
            }
        }
    }

    fields
}

// --- Python: class fields (type annotations) ---

fn extract_python_class_fields(
    root: Node<'_>,
    code: &str,
    type_name: &str,
) -> Result<Vec<TypeField>> {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        let kind = node.kind();

        if kind == "class_definition" || kind == "decorated_definition" {
            let class_node = if kind == "decorated_definition" {
                // Ищем class_definition внутри
                (0..node.child_count())
                    .filter_map(|j| node.child(j as u32))
                    .find(|c| c.kind() == "class_definition")
            } else {
                Some(node)
            };

            let Some(class_node) = class_node else {
                continue;
            };

            if let Some(name_node) = class_node.child_by_field_name("name") {
                if &code[name_node.byte_range()] == type_name {
                    return Ok(extract_fields_from_python_class(class_node, code));
                }
            }
        }
    }
    Ok(Vec::new())
}

fn collect_all_python_classes(
    root: Node<'_>,
    code: &str,
    results: &mut Vec<(String, Vec<TypeField>)>,
) {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        let kind = node.kind();

        let class_node = if kind == "decorated_definition" {
            (0..node.child_count())
                .filter_map(|j| node.child(j as u32))
                .find(|c| c.kind() == "class_definition")
        } else if kind == "class_definition" {
            Some(node)
        } else {
            None
        };

        let Some(class_node) = class_node else {
            continue;
        };

        if let Some(name_node) = class_node.child_by_field_name("name") {
            let name = code[name_node.byte_range()].to_string();
            let fields = extract_fields_from_python_class(class_node, code);
            if !fields.is_empty() {
                results.push((name, fields));
            }
        }
    }
}

fn extract_fields_from_python_class(class_node: Node<'_>, code: &str) -> Vec<TypeField> {
    let mut fields = Vec::new();

    let Some(body) = class_node.child_by_field_name("body") else {
        return fields;
    };

    for i in 0..body.child_count() {
        let Some(child) = body.child(i as u32) else { continue };

        // type: expression, e.g. `name: str`
        if child.kind() == "expression_statement" {
            if let Some(expr) = child.child(0 as u32) {
                if expr.kind() == "type" || expr.kind() == "assignment" {
                    // type annotation: `field: Type`
                    if let Some(name_node) = expr.child(0 as u32) {
                        if name_node.kind() == "identifier" {
                            let name = code[name_node.byte_range()].to_string();
                            if !name.starts_with('_') {
                                let field_type =
                                    expr.child(2 as u32).map(|t| code[t.byte_range()].to_string());
                                let normalized = normalize_field(&name);
                                fields.push(TypeField {
                                    name,
                                    field_type,
                                    normalized,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fields
}

fn extract_go_struct_fields(root: Node<'_>, code: &str, type_name: &str) -> Result<Vec<TypeField>> {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        if node.kind() == "type_declaration" {
            for j in 0..node.child_count() {
                let Some(spec) = node.child(j as u32) else { continue };
                if spec.kind() == "type_spec" {
                    if let Some(name_node) = spec.child_by_field_name("name") {
                        if &code[name_node.byte_range()] == type_name {
                            if let Some(type_node) = spec.child_by_field_name("type") {
                                if type_node.kind() == "struct_type" {
                                    return Ok(extract_fields_from_go_struct(type_node, code));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(Vec::new())
}

fn collect_all_go_structs(root: Node<'_>, code: &str, results: &mut Vec<(String, Vec<TypeField>)>) {
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        if node.kind() == "type_declaration" {
            for j in 0..node.child_count() {
                let Some(spec) = node.child(j as u32) else { continue };
                if spec.kind() == "type_spec" {
                    if let Some(name_node) = spec.child_by_field_name("name") {
                        let name = code[name_node.byte_range()].to_string();
                        if let Some(type_node) = spec.child_by_field_name("type") {
                            if type_node.kind() == "struct_type" {
                                let fields = extract_fields_from_go_struct(type_node, code);
                                if !fields.is_empty() {
                                    results.push((name, fields));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn extract_fields_from_go_struct(struct_node: Node<'_>, code: &str) -> Vec<TypeField> {
    let mut fields = Vec::new();

    // Go struct_type contains field_declaration_list
    if let Some(body) = struct_node.child_by_field_name("body") {
        extract_go_fields_from_body(body, code, &mut fields);
    } else {
        // Some tree-sitter-go versions: struct_type children include field_declaration directly
        for i in 0..struct_node.child_count() {
            let Some(child) = struct_node.child(i as u32) else {
                continue;
            };
            if child.kind() == "field_declaration_list" {
                extract_go_fields_from_body(child, code, &mut fields);
            }
        }
    }

    fields
}

fn extract_go_fields_from_body(body: Node<'_>, code: &str, fields: &mut Vec<TypeField>) {
    for i in 0..body.child_count() {
        let Some(child) = body.child(i as u32) else { continue };
        if child.kind() == "field_declaration" {
            // field_declaration: name type [tag]
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = code[name_node.byte_range()].to_string();
                let field_type = child
                    .child_by_field_name("type")
                    .map(|t| code[t.byte_range()].to_string());
                let normalized = normalize_field(&name);
                fields.push(TypeField {
                    name,
                    field_type,
                    normalized,
                });
            }
        }
    }
}

// --- Go: skeleton body collection ---
