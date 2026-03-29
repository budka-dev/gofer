#![allow(dead_code)]
use tree_sitter::Node;
use super::core::{ParserError, Result, SupportedLanguage};
use super::LANG_MANAGER;
use crate::models::chunk::TypeField;

/// Извлекает поля структур/классов и методы интерфейсов
pub fn parse_type_fields(
    code: &str,
    language: SupportedLanguage,
    type_name: &str,
) -> Result<Vec<TypeField>> {
    let lang = LANG_MANAGER.get_language(language.name())
        .ok_or_else(|| ParserError::UnsupportedLanguage(language.name().to_string()))?;
    
    let tree = crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&lang.language).map_err(|e| ParserError::LangManagerError(e.to_string()))?;
        parser.parse(code, None).ok_or(ParserError::ParseError)
    })?;
    let root = tree.root_node();

    match language.name() {
        "rust" => Ok(collect_rust_fields(root, code, type_name)),
        _ => Ok(Vec::new()),
    }
}

pub fn parse_all_type_fields(
    code: &str,
    language: SupportedLanguage,
) -> Result<std::collections::HashMap<String, Vec<TypeField>>> {
    let ts_lang_name = if language.name() == "vue" {
        "typescript"
    } else {
        language.name()
    };

    let lang = LANG_MANAGER.get_language(ts_lang_name)
        .ok_or_else(|| ParserError::UnsupportedLanguage(ts_lang_name.to_string()))?;
    
    let tree = crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&lang.language).map_err(|e| ParserError::LangManagerError(e.to_string()))?;
        parser.parse(code, None).ok_or(ParserError::ParseError)
    })?;
    let root = tree.root_node();

    match ts_lang_name {
        "rust" => Ok(collect_all_rust_fields(root, code)),
        _ => Ok(std::collections::HashMap::new()),
    }
}

fn collect_rust_fields(root: Node, code: &str, type_name: &str) -> Vec<TypeField> {
    let mut fields = Vec::new();
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        if node.kind() == "struct_item" {
            if let Some(name_node) = node.child_by_field_name("name") {
                if &code[name_node.byte_range()] == type_name {
                    if let Some(body) = node.child_by_field_name("body") {
                        for j in 0..body.child_count() {
                            let Some(field) = body.child(j as u32) else { continue };
                            if field.kind() == "field_declaration" {
                                if let Some(n) = field.child_by_field_name("name") {
                                    let name = code[n.byte_range()].to_string();
                                    fields.push(TypeField {
                                        normalized: normalize_field(&name),
                                        name,
                                        field_type: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fields
}

fn collect_all_rust_fields(root: Node, code: &str) -> std::collections::HashMap<String, Vec<TypeField>> {
    let mut result = std::collections::HashMap::new();
    for i in 0..root.child_count() {
        let Some(node) = root.child(i as u32) else { continue };
        if node.kind() == "struct_item" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let type_name = code[name_node.byte_range()].to_string();
                let mut fields = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    for j in 0..body.child_count() {
                        let Some(field) = body.child(j as u32) else { continue };
                        if field.kind() == "field_declaration" {
                            if let Some(n) = field.child_by_field_name("name") {
                                let name = code[n.byte_range()].to_string();
                                fields.push(TypeField {
                                    normalized: normalize_field(&name),
                                    name,
                                    field_type: None,
                                });
                            }
                        }
                    }
                }
                result.insert(type_name, fields);
            }
        }
    }
    result
}

pub fn normalize_field(field: &str) -> String {
    field.to_lowercase().replace(['_', '-'], "")
}
