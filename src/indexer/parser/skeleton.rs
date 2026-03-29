use regex::Regex;
use tree_sitter::Node;

use super::core::{ParserError, Result, SupportedLanguage};
use super::LANG_MANAGER;

// === Code Skeletonization ===

pub fn generate_skeleton(code: &str, language: SupportedLanguage) -> Result<String> {
    if language.name() == SupportedLanguage::VUE {
        return generate_vue_skeleton(code);
    }

    generate_skeleton_internal(code, language)
}

fn generate_skeleton_internal(code: &str, language: SupportedLanguage) -> Result<String> {
    let lang = LANG_MANAGER.get_language(language.name())
        .ok_or_else(|| ParserError::UnsupportedLanguage(language.name().to_string()))?;
    
    let tree = crate::indexer::parser::with_parser(|parser| {
        parser.set_language(&lang.language).map_err(|e| ParserError::LangManagerError(e.to_string()))?;
        parser.parse(code, None).ok_or(ParserError::ParseError)
    })?;
    let root = tree.root_node();

    let mut replacements: Vec<(usize, usize, &str)> = Vec::new();
    collect_body_ranges(root, code, &language, &mut replacements);

    replacements.sort_by_key(|r| r.0);
    let merged = merge_ranges(&replacements);

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

fn collect_body_ranges(
    node: Node<'_>,
    code: &str,
    language: &SupportedLanguage,
    replacements: &mut Vec<(usize, usize, &str)>,
) {
    match language.name() {
        SupportedLanguage::RUST => collect_rust_bodies(node, code, replacements),
        SupportedLanguage::TYPESCRIPT | SupportedLanguage::JAVASCRIPT => collect_ts_bodies(node, code, replacements),
        SupportedLanguage::PYTHON => collect_python_bodies(node, code, replacements),
        SupportedLanguage::GO => collect_go_bodies(node, code, replacements),
        _ => {}
    }
}

fn collect_rust_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();
    if kind == "function_item" {
        if let Some(body) = node.child_by_field_name("body") {
            if body.kind() == "block" {
                replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                return;
            }
        }
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_rust_bodies(child, _code, replacements);
        }
    }
}

fn collect_ts_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();
    match kind {
        "function_declaration" | "method_definition" | "arrow_function" => {
            if let Some(body) = node.child_by_field_name("body") {
                if body.kind() == "statement_block" {
                    replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                    return;
                }
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_ts_bodies(child, _code, replacements);
        }
    }
}

fn collect_python_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();
    if kind == "function_definition" {
        if let Some(body) = node.child_by_field_name("body") {
            replacements.push((body.start_byte(), body.end_byte(), "..."));
            return;
        }
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            collect_python_bodies(child, _code, replacements);
        }
    }
}

fn collect_go_bodies(node: Node<'_>, _code: &str, replacements: &mut Vec<(usize, usize, &str)>) {
    let kind = node.kind();
    match kind {
        "function_declaration" | "method_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                replacements.push((body.start_byte(), body.end_byte(), "{ /* ... */ }"));
                return;
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

fn merge_ranges<'a>(ranges: &[(usize, usize, &'a str)]) -> Vec<(usize, usize, &'a str)> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut merged = Vec::new();
    let mut current = ranges[0];
    for &next in &ranges[1..] {
        if next.0 >= current.1 {
            merged.push(current);
            current = next;
        }
    }
    merged.push(current);
    merged
}

fn generate_vue_skeleton(code: &str) -> Result<String> {
    let script_re = Regex::new(r"(?s)(<script[^>]*>\n?)(.*?)(</script>)").unwrap();
    if let Some(caps) = script_re.captures(code) {
        let prefix = caps.get(1).unwrap().as_str();
        let script_body = caps.get(2).unwrap();
        let suffix = caps.get(3).unwrap().as_str();
        let skeleton = generate_skeleton_internal(script_body.as_str(), SupportedLanguage(SupportedLanguage::TYPESCRIPT.to_string()))?;
        let mut result = code[..caps.get(0).unwrap().start()].to_string();
        result.push_str(prefix);
        result.push_str(&skeleton);
        result.push_str(suffix);
        result.push_str(&code[caps.get(0).unwrap().end()..]);
        Ok(result)
    } else {
        Ok(code.to_string())
    }
}
