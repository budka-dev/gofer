use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use tree_sitter::{Language, Node, Parser};

use super::{LanguageService, ToolDefinition};
use crate::storage::SqliteStorage;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PropInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prop_type: Option<String>,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VueComponentMeta {
    pub name: String,
    pub props: Vec<PropInfo>,
    pub emits: Vec<String>,
    pub slots: Vec<String>,
    pub has_script_setup: bool,
}

// ---------------------------------------------------------------------------
// VueService
// ---------------------------------------------------------------------------

pub struct VueService {
    #[allow(dead_code)]
    sqlite: SqliteStorage,
}

impl VueService {
    pub fn new(sqlite: SqliteStorage) -> Self {
        Self { sqlite }
    }
}

#[async_trait::async_trait]
impl LanguageService for VueService {
    fn name(&self) -> &str {
        "vue"
    }

    fn is_applicable(&self, root: &Path) -> bool {
        let pkg = root.join("package.json");
        if !pkg.exists() {
            return false;
        }
        // Quick check: does package.json mention "vue"?
        if let Ok(content) = std::fs::read_to_string(&pkg) {
            return content.contains("\"vue\"");
        }
        false
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        vec![
            // --- Group 1: Component Interface ---
            ToolDefinition {
                name: "vue_get_meta".into(),
                description: "Extract Vue component contract: props, emits, slots. Supports both <script setup> (defineProps/defineEmits) and Options API.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to the .vue file"
                        }
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                name: "vue_read_section".into(),
                description: "Read a specific section of a Vue SFC: template, script, or style. Saves tokens by returning only what you need.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path to the .vue file"
                        },
                        "section": {
                            "type": "string",
                            "enum": ["template", "script", "style"],
                            "description": "Which SFC section to return"
                        }
                    },
                    "required": ["path", "section"]
                }),
            },
            // --- Group 2: Component Graph ---
            ToolDefinition {
                name: "vue_find_usages".into(),
                description: "Find all files that import or use a given Vue component (handles PascalCase/kebab-case normalization).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "component_name": {
                            "type": "string",
                            "description": "Component name in PascalCase (e.g. 'UserCard')"
                        }
                    },
                    "required": ["component_name"]
                }),
            },
            ToolDefinition {
                name: "vue_resolve_component".into(),
                description: "Resolve a component tag name to the file it is defined in, by analysing imports in a given context file.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "tag_name": {
                            "type": "string",
                            "description": "Component tag as used in template (e.g. 'UserCard' or 'user-card')"
                        },
                        "context_path": {
                            "type": "string",
                            "description": "Relative path of the file where the tag appears"
                        }
                    },
                    "required": ["tag_name", "context_path"]
                }),
            },
            // --- Group 3: App Structure ---
            ToolDefinition {
                name: "vue_router_map".into(),
                description: "Parse Vue Router configuration and return the route map: URL path -> component file.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "vue_pinia_stores".into(),
                description: "Find all Pinia stores (defineStore) in the project and list their names, state fields, and actions.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        ]
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        ctx: &crate::daemon::tools::ToolContext,
    ) -> Result<String> {
        let root = ctx.root_path.as_path();
        match name {
            "vue_get_meta" => self.tool_get_meta(args, root).await,
            "vue_read_section" => self.tool_read_section(args, root).await,
            "vue_find_usages" => self.tool_find_usages(args, root).await,
            "vue_resolve_component" => self.tool_resolve_component(args, root).await,
            "vue_router_map" => self.tool_router_map(root).await,
            "vue_pinia_stores" => self.tool_pinia_stores(root).await,
            _ => Err(anyhow::anyhow!("Unknown Vue tool: {}", name)),
        }
    }
}

// ---------------------------------------------------------------------------
// SFC section extraction helpers (tree-sitter-html based)
// ---------------------------------------------------------------------------

/// Extract inner content of a top-level SFC section (<script>, <template>, <style>)
/// using tree-sitter-html. Falls back to regex if parsing fails.
fn extract_section(content: &str, tag: &str) -> Option<String> {
    if let Some(body) = extract_section_ts_html(content, tag) {
        return Some(body);
    }
    // Regex fallback
    let pattern = format!(r"(?s)<{tag}[^>]*>(.*?)</{tag}>");
    let re = Regex::new(&pattern).ok()?;
    re.captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn extract_section_with_attrs(content: &str, tag: &str) -> Option<(String, String)> {
    if let Some(result) = extract_section_with_attrs_ts_html(content, tag) {
        return Some(result);
    }
    let pattern = format!(r"(?s)<{tag}([^>]*)>(.*?)</{tag}>");
    let re = Regex::new(&pattern).ok()?;
    re.captures(content).map(|c| {
        let attrs = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
        let body = c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
        (attrs, body)
    })
}

fn has_script_setup(content: &str) -> bool {
    if let Some((attrs, _)) = extract_section_with_attrs_ts_html(content, "script") {
        return attrs.contains("setup");
    }
    let re = Regex::new(r"<script[^>]*\bsetup\b[^>]*>").unwrap();
    re.is_match(content)
}

/// tree-sitter-html based section extraction — more robust than regex for nested tags.
fn extract_section_ts_html(content: &str, tag: &str) -> Option<String> {
    let (_, body) = extract_section_with_attrs_ts_html(content, tag)?;
    Some(body)
}

fn extract_section_with_attrs_ts_html(content: &str, tag: &str) -> Option<(String, String)> {
    let mut parser = Parser::new();
    let html_lang: Language = tree_sitter_html::LANGUAGE.into();
    parser.set_language(&html_lang).ok()?;
    let tree = parser.parse(content, None)?;
    let root = tree.root_node();
    let element_kind = format!("{}_element", tag);

    for i in 0..root.child_count() {
        let child = root.child(i as u32)?;
        if child.kind() != element_kind && child.kind() != "element" {
            continue;
        }
        // Verify tag name matches via start_tag child
        if child.kind() == "element" {
            let start_tag = child.child_by_field_name("start_tag").or_else(|| {
                (0..child.child_count())
                    .filter_map(|j| child.child(j as u32))
                    .find(|n| n.kind() == "start_tag")
            });
            if let Some(st) = start_tag {
                let tag_name = (0..st.child_count())
                    .filter_map(|j| st.child(j as u32))
                    .find(|n| n.kind() == "tag_name")
                    .and_then(|n| n.utf8_text(content.as_bytes()).ok());
                if tag_name != Some(tag) {
                    continue;
                }
            } else {
                continue;
            }
        }

        // Extract attributes from start tag
        let mut attrs = String::new();
        for j in 0..child.child_count() {
            if let Some(inner) = child.child(j as u32) {
                if inner.kind() == "start_tag" {
                    let full_tag = inner.utf8_text(content.as_bytes()).ok().unwrap_or("");
                    // Attrs = everything between <tag and >
                    if let Some(space_pos) = full_tag.find(char::is_whitespace) {
                        attrs = full_tag[space_pos..].trim_end_matches('>').to_string();
                    }
                }
            }
        }

        // Extract body (raw_text for script/style, or text content for template)
        for j in 0..child.child_count() {
            if let Some(inner) = child.child(j as u32) {
                if inner.kind() == "raw_text" || inner.kind() == "text" {
                    let body = inner.utf8_text(content.as_bytes()).ok()?.to_string();
                    return Some((attrs, body));
                }
            }
        }

        // For template: the body is everything between start_tag and end_tag
        let start_tag_end = (0..child.child_count())
            .filter_map(|j| child.child(j as u32))
            .find(|n| n.kind() == "start_tag")
            .map(|n| n.end_byte());
        let end_tag_start = (0..child.child_count())
            .filter_map(|j| child.child(j as u32))
            .find(|n| n.kind() == "end_tag")
            .map(|n| n.start_byte());

        if let (Some(start), Some(end)) = (start_tag_end, end_tag_start) {
            if start < end && end <= content.len() {
                let body = content[start..end].to_string();
                return Some((attrs, body));
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Convert PascalCase to kebab-case: "UserCard" -> "user-card"
fn pascal_to_kebab(name: &str) -> String {
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert kebab-case to PascalCase: "user-card" -> "UserCard"
fn kebab_to_pascal(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

/// Derive component name from file path: "src/components/UserCard.vue" -> "UserCard"
fn component_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_prefix()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tree-sitter-typescript based meta extraction
// ---------------------------------------------------------------------------

/// Extract defineProps using tree-sitter-typescript AST parsing
fn extract_define_props(script: &str) -> Vec<PropInfo> {
    let mut parser = Parser::new();
    let ts_lang: Language = tree_sitter_typescript::LANGUAGE_TSX.into();

    if parser.set_language(&ts_lang).is_err() {
        return extract_define_props_fallback(script);
    }

    let Some(tree) = parser.parse(script, None) else {
        return extract_define_props_fallback(script);
    };

    let root = tree.root_node();
    let mut props = Vec::new();

    // Strategy: Find call_expression nodes with identifier "defineProps"
    find_define_props_calls(root, script.as_bytes(), &mut props);

    if props.is_empty() {
        // Try Options API: export default { props: { ... } }
        find_options_api_props(root, script.as_bytes(), &mut props);
    }

    props
}

fn find_define_props_calls(node: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // Composition API: defineProps<{ ... }>() or defineProps({ ... })
    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let func_text = func_node.utf8_text(source).unwrap_or("");

            // Check if this is defineProps or withDefaults
            if func_text == "defineProps" {
                extract_props_from_call(node, source, props);
            } else if func_text == "withDefaults" {
                extract_props_from_with_defaults(node, source, props);
            }
        }
    }

    // Recurse through children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            find_define_props_calls(child, source, props);
        }
    }
}

fn extract_props_from_call(node: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // defineProps<{ msg: string, count?: number }>()
    if let Some(type_args) = node.child_by_field_name("type_arguments") {
        extract_props_from_type_object(type_args, source, props);
        return;
    }

    // defineProps({ msg: String, count: Number })
    if let Some(args_node) = node.child_by_field_name("arguments") {
        for i in 0..args_node.child_count() {
            if let Some(arg) = args_node.child(i as u32) {
                if arg.kind() == "object" {
                    extract_props_from_runtime_object(arg, source, props);
                } else if arg.kind() == "array" {
                    extract_props_from_array(arg, source, props);
                }
            }
        }
    }
}

fn extract_props_from_with_defaults(node: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // withDefaults(defineProps<{ ... }>(), { ... })
    if let Some(args_node) = node.child_by_field_name("arguments") {
        let mut define_props_node = None;
        let mut defaults_node = None;

        for i in 0..args_node.child_count() {
            if let Some(arg) = args_node.child(i as u32) {
                if arg.kind() == "call_expression" {
                    if let Some(func) = arg.child_by_field_name("function") {
                        if func.utf8_text(source).unwrap_or("") == "defineProps" {
                            define_props_node = Some(arg);
                        }
                    }
                } else if arg.kind() == "object" && define_props_node.is_some() {
                    defaults_node = Some(arg);
                }
            }
        }

        if let Some(dp_node) = define_props_node {
            extract_props_from_call(dp_node, source, props);
        }

        if let Some(def_node) = defaults_node {
            apply_defaults_from_object(props, def_node, source);
        }
    }
}

fn extract_props_from_type_object(type_args: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // <{ msg: string, count?: number }>
    for i in 0..type_args.child_count() {
        if let Some(child) = type_args.child(i as u32) {
            if child.kind() == "object_type" || child.kind() == "type_literal" {
                extract_props_from_type_literal(child, source, props);
            }
        }
    }
}

fn extract_props_from_type_literal(obj: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // Parse property_signature nodes: msg: string, count?: number
    for i in 0..obj.child_count() {
        if let Some(prop) = obj.child(i as u32) {
            if prop.kind() == "property_signature" {
                let mut name = String::new();
                let mut prop_type = None;
                let mut required = true;

                // Extract name
                if let Some(name_node) = prop.child_by_field_name("name") {
                    name = name_node.utf8_text(source).unwrap_or("").to_string();
                }

                // Check if optional (has ? token)
                for j in 0..prop.child_count() {
                    if let Some(child) = prop.child(j as u32) {
                        if child.kind() == "?" {
                            required = false;
                        } else if child.kind() == "type_annotation" {
                            if let Some(type_node) = child.child(1 as u32) {
                                prop_type =
                                    Some(type_node.utf8_text(source).unwrap_or("any").to_string());
                            }
                        }
                    }
                }

                if !name.is_empty() {
                    props.push(PropInfo {
                        name,
                        prop_type,
                        required,
                        default: None,
                    });
                }
            }
        }
    }
}

fn extract_props_from_runtime_object(obj: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // defineProps({ msg: String, count: { type: Number, required: true } })
    for i in 0..obj.child_count() {
        if let Some(pair) = obj.child(i as u32) {
            if pair.kind() == "pair" {
                let mut name = String::new();
                let mut prop_type = None;
                let mut required = false;
                let mut default = None;

                if let Some(key) = pair.child_by_field_name("key") {
                    name = key.utf8_text(source).unwrap_or("").to_string();
                }

                if let Some(value) = pair.child_by_field_name("value") {
                    let value_text = value.utf8_text(source).unwrap_or("");

                    if value.kind() == "identifier" {
                        // Simple: msg: String
                        prop_type = Some(value_text.to_string());
                    } else if value.kind() == "object" {
                        // Complex: { type: Number, required: true, default: 0 }
                        for j in 0..value.child_count() {
                            if let Some(inner_pair) = value.child(j as u32) {
                                if inner_pair.kind() == "pair" {
                                    if let Some(inner_key) = inner_pair.child_by_field_name("key") {
                                        let key_text = inner_key.utf8_text(source).unwrap_or("");
                                        if let Some(inner_val) =
                                            inner_pair.child_by_field_name("value")
                                        {
                                            let val_text =
                                                inner_val.utf8_text(source).unwrap_or("");

                                            match key_text {
                                                "type" => prop_type = Some(val_text.to_string()),
                                                "required" => required = val_text == "true",
                                                "default" => default = Some(val_text.to_string()),
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !name.is_empty() {
                    props.push(PropInfo {
                        name,
                        prop_type,
                        required,
                        default,
                    });
                }
            }
        }
    }
}

fn extract_props_from_array(arr: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // defineProps(['msg', 'count'])
    for i in 0..arr.child_count() {
        if let Some(item) = arr.child(i as u32) {
            if item.kind() == "string" {
                let text = item.utf8_text(source).unwrap_or("");
                let name = text.trim_matches(|c| c == '"' || c == '\'');
                props.push(PropInfo {
                    name: name.to_string(),
                    prop_type: None,
                    required: false,
                    default: None,
                });
            }
        }
    }
}

fn apply_defaults_from_object(props: &mut [PropInfo], defaults_obj: Node, source: &[u8]) {
    for i in 0..defaults_obj.child_count() {
        if let Some(pair) = defaults_obj.child(i as u32) {
            if pair.kind() == "pair" {
                if let Some(key) = pair.child_by_field_name("key") {
                    let key_text = key.utf8_text(source).unwrap_or("");
                    if let Some(value) = pair.child_by_field_name("value") {
                        let val_text = value.utf8_text(source).unwrap_or("");

                        if let Some(prop) = props.iter_mut().find(|p| p.name == key_text) {
                            prop.default = Some(val_text.to_string());
                            prop.required = false;
                        }
                    }
                }
            }
        }
    }
}

fn find_options_api_props(node: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    // export default { props: { ... } } or { props: ['a', 'b'] }
    if node.kind() == "export_statement" {
        if let Some(value) = node.child_by_field_name("value") {
            find_props_in_object(value, source, props);
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            find_options_api_props(child, source, props);
        }
    }
}

fn find_props_in_object(node: Node, source: &[u8], props: &mut Vec<PropInfo>) {
    if node.kind() == "object" {
        for i in 0..node.child_count() {
            if let Some(pair) = node.child(i as u32) {
                if pair.kind() == "pair" {
                    if let Some(key) = pair.child_by_field_name("key") {
                        if key.utf8_text(source).unwrap_or("") == "props" {
                            if let Some(value) = pair.child_by_field_name("value") {
                                if value.kind() == "object" {
                                    extract_props_from_runtime_object(value, source, props);
                                } else if value.kind() == "array" {
                                    extract_props_from_array(value, source, props);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Regex fallback for defineProps extraction
fn extract_define_props_fallback(script: &str) -> Vec<PropInfo> {
    let mut props = Vec::new();

    // Simple pattern: defineProps<{ ... }>() or defineProps({ ... })
    if let Some(start) = script.find("defineProps") {
        let after = &script[start..];

        // Try to extract type argument or runtime object
        if let Some(type_start) = after.find('<') {
            if let Some(type_end) = after[type_start..].find('>') {
                let type_body = &after[type_start + 1..type_start + type_end];
                // Very simple regex parsing
                let field_re = Regex::new(r"(\w+)\s*(\?)?:\s*([^;\n,]+)").unwrap();
                for cap in field_re.captures_iter(type_body) {
                    props.push(PropInfo {
                        name: cap[1].to_string(),
                        prop_type: Some(cap[3].trim().to_string()),
                        required: cap.get(2).is_none(),
                        default: None,
                    });
                }
            }
        }
    }

    props
}

/// Extract defineEmits using tree-sitter-typescript AST
fn extract_define_emits(script: &str) -> Vec<String> {
    let mut parser = Parser::new();
    let ts_lang: Language = tree_sitter_typescript::LANGUAGE_TSX.into();

    if parser.set_language(&ts_lang).is_err() {
        return extract_define_emits_fallback(script);
    }

    let Some(tree) = parser.parse(script, None) else {
        return extract_define_emits_fallback(script);
    };

    let root = tree.root_node();
    let mut emits = Vec::new();

    find_define_emits_calls(root, script.as_bytes(), &mut emits);

    if emits.is_empty() {
        find_options_api_emits(root, script.as_bytes(), &mut emits);
    }

    emits
}

fn find_define_emits_calls(node: Node, source: &[u8], emits: &mut Vec<String>) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            if func.utf8_text(source).unwrap_or("") == "defineEmits" {
                // defineEmits<{ ... }>() or defineEmits(['submit', 'cancel'])

                // Check for type arguments first
                if let Some(type_args) = node.child_by_field_name("type_arguments") {
                    extract_emits_from_type(type_args, source, emits);
                }

                // Check for array argument
                if let Some(args) = node.child_by_field_name("arguments") {
                    for i in 0..args.child_count() {
                        if let Some(arg) = args.child(i as u32) {
                            if arg.kind() == "array" {
                                extract_emits_from_array(arg, source, emits);
                            }
                        }
                    }
                }
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            find_define_emits_calls(child, source, emits);
        }
    }
}

fn extract_emits_from_type(type_args: Node, source: &[u8], emits: &mut Vec<String>) {
    // <{ (e: 'submit', val: any): void; ... }> or <{ submit: [...] }>
    for i in 0..type_args.child_count() {
        if let Some(child) = type_args.child(i as u32) {
            if child.kind() == "object_type" || child.kind() == "type_literal" {
                extract_emits_from_type_literal(child, source, emits);
            }
        }
    }
}

fn extract_emits_from_type_literal(obj: Node, source: &[u8], emits: &mut Vec<String>) {
    for i in 0..obj.child_count() {
        if let Some(item) = obj.child(i as u32) {
            // Look for string literals in function signatures or property names
            if item.kind() == "call_signature" || item.kind() == "method_signature" {
                extract_strings_from_node(item, source, emits);
            } else if item.kind() == "property_signature" {
                if let Some(name) = item.child_by_field_name("name") {
                    let name_text = name.utf8_text(source).unwrap_or("");
                    if !name_text.is_empty() && name_text != "e" {
                        emits.push(name_text.to_string());
                    }
                }
            }
        }
    }
}

fn extract_strings_from_node(node: Node, source: &[u8], emits: &mut Vec<String>) {
    if node.kind() == "string" {
        let text = node.utf8_text(source).unwrap_or("");
        let clean = text.trim_matches(|c| c == '"' || c == '\'');
        if !clean.is_empty() {
            emits.push(clean.to_string());
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            extract_strings_from_node(child, source, emits);
        }
    }
}

fn extract_emits_from_array(arr: Node, source: &[u8], emits: &mut Vec<String>) {
    for i in 0..arr.child_count() {
        if let Some(item) = arr.child(i as u32) {
            if item.kind() == "string" {
                let text = item.utf8_text(source).unwrap_or("");
                let clean = text.trim_matches(|c| c == '"' || c == '\'');
                emits.push(clean.to_string());
            }
        }
    }
}

fn find_options_api_emits(node: Node, source: &[u8], emits: &mut Vec<String>) {
    if node.kind() == "export_statement" {
        if let Some(value) = node.child_by_field_name("value") {
            find_emits_in_object(value, source, emits);
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            find_options_api_emits(child, source, emits);
        }
    }
}

fn find_emits_in_object(node: Node, source: &[u8], emits: &mut Vec<String>) {
    if node.kind() == "object" {
        for i in 0..node.child_count() {
            if let Some(pair) = node.child(i as u32) {
                if pair.kind() == "pair" {
                    if let Some(key) = pair.child_by_field_name("key") {
                        if key.utf8_text(source).unwrap_or("") == "emits" {
                            if let Some(value) = pair.child_by_field_name("value") {
                                if value.kind() == "array" {
                                    extract_emits_from_array(value, source, emits);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn extract_define_emits_fallback(script: &str) -> Vec<String> {
    let mut emits = Vec::new();

    // Simple regex fallback
    if let Some(start) = script.find("defineEmits") {
        let after = &script[start..];
        if let Some(arr_start) = after.find('[') {
            if let Some(arr_end) = after[arr_start..].find(']') {
                let arr_body = &after[arr_start + 1..arr_start + arr_end];
                let string_re = Regex::new(r#"['"]([^'"]+)['"]"#).unwrap();
                for cap in string_re.captures_iter(arr_body) {
                    emits.push(cap[1].to_string());
                }
            }
        }
    }

    emits
}

/// Extract slots from template (simple regex is fine here)
fn extract_slots(template: &str) -> Vec<String> {
    let mut slots = HashSet::new();

    // Named slots: <slot name="header" />
    let named_re = Regex::new(r#"<slot\s[^>]*name\s*=\s*["']([^"']+)["']"#).unwrap();
    for cap in named_re.captures_iter(template) {
        slots.insert(cap[1].to_string());
    }

    // Default slot: bare <slot> or <slot /> without name
    let bare_re = Regex::new(r"<slot\s*/?\s*>").unwrap();
    if bare_re.is_match(template) {
        slots.insert("default".to_string());
    }

    // Also <slot> without name attr but with other attrs
    let slot_tag_re = Regex::new(r"<slot\b([^>]*)>").unwrap();
    for cap in slot_tag_re.captures_iter(template) {
        let attrs = &cap[1];
        if !attrs.contains("name=") {
            slots.insert("default".to_string());
        }
    }

    let mut result: Vec<String> = slots.into_iter().collect();
    result.sort();
    result
}

// ---------------------------------------------------------------------------
// Full meta extraction
// ---------------------------------------------------------------------------

fn extract_component_meta(content: &str, file_path: &str) -> VueComponentMeta {
    let name = component_name_from_path(file_path);
    let setup = has_script_setup(content);

    let script = extract_section(content, "script").unwrap_or_default();
    let template = extract_section(content, "template").unwrap_or_default();

    let props = extract_define_props(&script);
    let emits = extract_define_emits(&script);
    let slots = extract_slots(&template);

    VueComponentMeta {
        name,
        props,
        emits,
        slots,
        has_script_setup: setup,
    }
}

// ---------------------------------------------------------------------------
// File walker: find .vue and .ts files
// ---------------------------------------------------------------------------

fn walk_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_files_recursive(root, extensions, &mut files);
    files
}

fn walk_files_recursive(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "node_modules" || name == "dist" || name == "build"
            {
                continue;
            }
            walk_files_recursive(&path, extensions, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if extensions.contains(&ext) {
                out.push(path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

impl VueService {
    /// `vue_get_meta` — extract component props, emits, slots
    async fn tool_get_meta(&self, args: Value, root: &Path) -> Result<String> {
        let rel_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'path' is required"))?;

        let abs_path = root.join(rel_path);
        if !abs_path.exists() {
            return Err(anyhow::anyhow!("File not found: {}", rel_path));
        }

        let content = tokio::fs::read_to_string(&abs_path).await?;
        let meta = extract_component_meta(&content, rel_path);

        let mut out = format!("# Component: `{}`\n\n", meta.name);
        out.push_str(&format!(
            "**API style:** {}\n\n",
            if meta.has_script_setup {
                "Composition API (`<script setup>`)"
            } else {
                "Options API"
            }
        ));

        // Props
        out.push_str("## Props\n\n");
        if meta.props.is_empty() {
            out.push_str("*No props defined.*\n\n");
        } else {
            out.push_str("| Name | Type | Required | Default |\n");
            out.push_str("|------|------|----------|---------|\n");
            for p in &meta.props {
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    p.name,
                    p.prop_type.as_deref().unwrap_or("-"),
                    if p.required { "yes" } else { "no" },
                    p.default.as_deref().unwrap_or("-"),
                ));
            }
            out.push('\n');
        }

        // Emits
        out.push_str("## Emits\n\n");
        if meta.emits.is_empty() {
            out.push_str("*No emits defined.*\n\n");
        } else {
            for e in &meta.emits {
                out.push_str(&format!("- `{}`\n", e));
            }
            out.push('\n');
        }

        // Slots
        out.push_str("## Slots\n\n");
        if meta.slots.is_empty() {
            out.push_str("*No slots found in template.*\n\n");
        } else {
            for s in &meta.slots {
                out.push_str(&format!("- `{}`\n", s));
            }
            out.push('\n');
        }

        // Also append JSON for programmatic use
        out.push_str("## JSON Schema\n\n");
        out.push_str("```json\n");
        out.push_str(&serde_json::to_string_pretty(&meta)?);
        out.push_str("\n```\n");

        Ok(out)
    }

    /// `vue_read_section` — return a single SFC section
    async fn tool_read_section(&self, args: Value, root: &Path) -> Result<String> {
        let rel_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'path' is required"))?;
        let section = args
            .get("section")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'section' is required"))?;

        let abs_path = root.join(rel_path);
        if !abs_path.exists() {
            return Err(anyhow::anyhow!("File not found: {}", rel_path));
        }

        let content = tokio::fs::read_to_string(&abs_path).await?;

        let (tag, lang_hint) = match section {
            "template" => ("template", "html"),
            "script" => ("script", "typescript"),
            "style" => ("style", "css"),
            _ => {
                return Err(anyhow::anyhow!(
                    "Invalid section: '{}'. Use template, script, or style.",
                    section
                ))
            }
        };

        match extract_section_with_attrs(&content, tag) {
            Some((attrs, body)) => {
                let lang = if attrs.contains("lang=\"scss\"") {
                    "scss"
                } else if attrs.contains("lang=\"sass\"") {
                    "sass"
                } else if attrs.contains("lang=\"less\"") {
                    "less"
                } else if attrs.contains("lang=\"ts\"") || attrs.contains("setup") {
                    "typescript"
                } else if attrs.contains("lang=\"pug\"") {
                    "pug"
                } else {
                    lang_hint
                };

                let setup_marker = if tag == "script" && attrs.contains("setup") {
                    " (script setup)"
                } else {
                    ""
                };

                Ok(format!(
                    "# `{}` — <{}{}>{}\n\n```{}\n{}\n```\n",
                    rel_path,
                    tag,
                    attrs,
                    setup_marker,
                    lang,
                    body.trim()
                ))
            }
            None => Ok(format!("No `<{}>` section found in `{}`.\n", tag, rel_path)),
        }
    }

    /// `vue_find_usages` — find component imports and template usages
    async fn tool_find_usages(&self, args: Value, root: &Path) -> Result<String> {
        let component_name = args
            .get("component_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'component_name' is required"))?;

        let pascal = if component_name.contains('-') {
            kebab_to_pascal(component_name)
        } else {
            component_name.to_string()
        };
        let kebab = pascal_to_kebab(&pascal);

        let vue_files = walk_files(root, &["vue", "ts", "tsx", "js", "jsx"]);

        let mut usages: Vec<(String, Vec<String>)> = Vec::new();

        for file_path in &vue_files {
            let Ok(content) = tokio::fs::read_to_string(file_path).await else {
                continue;
            };

            let rel = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .display()
                .to_string();

            let mut reasons = Vec::new();

            // Check imports: import UserCard from, import { UserCard }
            if content.contains(&pascal) {
                let import_re =
                    Regex::new(&format!(r"import\s+.*\b{}\b", regex::escape(&pascal))).unwrap();
                if import_re.is_match(&content) {
                    reasons.push("import".to_string());
                }
            }

            // Check template usage: <UserCard> or <user-card>
            if let Some(template) = extract_section(&content, "template") {
                // PascalCase tag
                if template.contains(&format!("<{}", &pascal))
                    || template.contains(&format!("</{}", &pascal))
                {
                    reasons.push("template (PascalCase)".to_string());
                }
                // kebab-case tag
                if template.contains(&format!("<{}", &kebab))
                    || template.contains(&format!("</{}", &kebab))
                {
                    reasons.push("template (kebab-case)".to_string());
                }
            }

            if !reasons.is_empty() {
                usages.push((rel, reasons));
            }
        }

        let mut out = format!("# Usages of `{}`\n\n", pascal);

        if usages.is_empty() {
            out.push_str("No usages found in the project.\n");
        } else {
            out.push_str(&format!("Found in **{}** file(s):\n\n", usages.len()));
            for (path, reasons) in &usages {
                out.push_str(&format!("- `{}` ({})\n", path, reasons.join(", ")));
            }
        }

        Ok(out)
    }

    /// `vue_resolve_component` — resolve tag to source file via imports
    async fn tool_resolve_component(&self, args: Value, root: &Path) -> Result<String> {
        let tag_name = args
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'tag_name' is required"))?;
        let context_path = args
            .get("context_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'context_path' is required"))?;

        let pascal = if tag_name.contains('-') {
            kebab_to_pascal(tag_name)
        } else {
            tag_name.to_string()
        };

        let abs_context = root.join(context_path);
        if !abs_context.exists() {
            return Err(anyhow::anyhow!("Context file not found: {}", context_path));
        }

        let content = tokio::fs::read_to_string(&abs_context).await?;
        let script = extract_section(&content, "script").unwrap_or(content.clone());

        // Find import of this component
        let import_re = Regex::new(&format!(
            r#"import\s+{}\s+from\s+['"](.*?)['"]"#,
            regex::escape(&pascal)
        ))
        .unwrap();

        let mut out = format!("# Resolve: `<{}>`\n\n", tag_name);
        out.push_str(&format!("**Context:** `{}`\n\n", context_path));

        if let Some(caps) = import_re.captures(&script) {
            let import_path = &caps[1];
            out.push_str(&format!("**Import path:** `{}`\n", import_path));

            // Try to resolve to actual file
            let context_dir = abs_context.parent().unwrap_or(root);
            let resolved = if import_path.starts_with('.') {
                // Relative import
                let candidate = context_dir.join(import_path);
                resolve_vue_import(root, &candidate)
            } else if import_path.starts_with('@') || import_path.starts_with('~') {
                // Alias — typically @/ -> src/
                let stripped = import_path
                    .trim_start_matches('@')
                    .trim_start_matches('~')
                    .trim_start_matches('/');
                let candidate = root.join("src").join(stripped);
                resolve_vue_import(root, &candidate)
            } else {
                None
            };

            if let Some(resolved_path) = resolved {
                let rel = resolved_path
                    .strip_prefix(root)
                    .unwrap_or(&resolved_path)
                    .display()
                    .to_string();
                out.push_str(&format!("**Resolved file:** `{}`\n", rel));
            } else {
                out.push_str(&format!(
                    "*Could not resolve `{}` to a file on disk.*\n",
                    import_path
                ));
            }
        } else {
            out.push_str("*No direct import found in script block.*\n\n");

            // Check if it could be a globally registered component
            out.push_str("Searching for global registration...\n\n");
            let vue_files = walk_files(root, &["vue"]);
            let mut candidates: Vec<String> = Vec::new();
            for f in &vue_files {
                let fname = f.file_prefix().and_then(|s| s.to_str()).unwrap_or("");
                if fname == pascal || kebab_to_pascal(fname) == pascal {
                    let rel = f.strip_prefix(root).unwrap_or(f).display().to_string();
                    candidates.push(rel);
                }
            }

            if candidates.is_empty() {
                out.push_str("*No matching .vue files found.*\n");
            } else {
                out.push_str("**Possible matches (by filename):**\n\n");
                for c in &candidates {
                    out.push_str(&format!("- `{}`\n", c));
                }
            }
        }

        Ok(out)
    }

    /// `vue_router_map` — parse Vue Router configuration
    async fn tool_router_map(&self, root: &Path) -> Result<String> {
        // Common router file locations
        let candidates = [
            "src/router/index.ts",
            "src/router/index.js",
            "src/router.ts",
            "src/router.js",
        ];

        let mut router_file = None;
        for c in &candidates {
            let path = root.join(c);
            if path.exists() {
                router_file = Some((c.to_string(), path));
                break;
            }
        }

        let Some((rel_path, abs_path)) = router_file else {
            return Ok("# Vue Router Map\n\nNo router file found. Searched:\n- src/router/index.ts\n- src/router/index.js\n- src/router.ts\n- src/router.js\n".into());
        };

        let content = tokio::fs::read_to_string(&abs_path).await?;

        let mut out = format!("# Vue Router Map\n\n**File:** `{}`\n\n", rel_path);
        out.push_str("| Path | Component | Name |\n");
        out.push_str("|------|-----------|------|\n");

        // Parse route definitions using regex (good enough for router config)
        let route_re = Regex::new(r#"(?s)\{\s*path\s*:\s*['"]([^'"]+)['"]\s*,([^}]*)\}"#).unwrap();

        let name_re = Regex::new(r#"name\s*:\s*['"]([^'"]+)['"]"#).unwrap();
        let component_re = Regex::new(r#"component\s*:\s*(\w+)"#).unwrap();
        let lazy_re = Regex::new(r#"import\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();

        let mut found = 0;
        for cap in route_re.captures_iter(&content) {
            let path = &cap[1];
            let rest = &cap[2];

            let name = name_re
                .captures(rest)
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| "-".into());

            let component = component_re
                .captures(rest)
                .map(|c| c[1].to_string())
                .or_else(|| {
                    lazy_re
                        .captures(rest)
                        .map(|c| format!("() => import('{}')", &c[1]))
                })
                .unwrap_or_else(|| "-".into());

            out.push_str(&format!("| `{}` | `{}` | {} |\n", path, component, name));
            found += 1;
        }

        if found == 0 {
            out.push_str("| *No routes parsed* | | |\n");
            out.push_str("\n*Router file exists but could not extract route definitions.*\n");
        }

        Ok(out)
    }

    /// `vue_pinia_stores` — find Pinia stores
    async fn tool_pinia_stores(&self, root: &Path) -> Result<String> {
        let ts_files = walk_files(root, &["ts", "js"]);

        let mut out = String::from("# Pinia Stores\n\n");

        let define_re = Regex::new(r#"defineStore\s*\(\s*['"](\w+)['"]"#).unwrap();
        let state_re =
            Regex::new(r"(?s)state\s*:\s*\(\s*\)\s*(?::\s*\w+\s*)?=>\s*\(\s*\{(.*?)\}\s*\)")
                .unwrap();
        let ref_state_re = Regex::new(r"(?:const|let)\s+(\w+)\s*=\s*ref\s*[<(]").unwrap();
        let action_re =
            Regex::new(r"(?:function\s+(\w+)|(?:const|let)\s+(\w+)\s*=\s*(?:async\s*)?\()")
                .unwrap();
        let field_re = Regex::new(r"(\w+)\s*:").unwrap();

        let mut store_count = 0;

        for file_path in &ts_files {
            let Ok(content) = tokio::fs::read_to_string(file_path).await else {
                continue;
            };

            if !content.contains("defineStore") {
                continue;
            }

            let rel = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .display()
                .to_string();

            for cap in define_re.captures_iter(&content) {
                let store_name = &cap[1];
                store_count += 1;

                out.push_str(&format!("## `{}` (`{}`)\n\n", store_name, rel));

                // Try to extract state fields (Options API style)
                if let Some(state_cap) = state_re.captures(&content) {
                    let state_body = &state_cap[1];
                    let fields: Vec<String> = field_re
                        .captures_iter(state_body)
                        .map(|c| c[1].to_string())
                        .collect();

                    if !fields.is_empty() {
                        out.push_str("**State:**\n");
                        for f in &fields {
                            out.push_str(&format!("- `{}`\n", f));
                        }
                        out.push('\n');
                    }
                }

                // Composition API: look for ref() declarations
                let refs: Vec<String> = ref_state_re
                    .captures_iter(&content)
                    .map(|c| c[1].to_string())
                    .collect();
                if !refs.is_empty() {
                    out.push_str("**Reactive State (ref):**\n");
                    for r in &refs {
                        out.push_str(&format!("- `{}`\n", r));
                    }
                    out.push('\n');
                }

                // Look for action-like functions (heuristic)
                let actions: Vec<String> = action_re
                    .captures_iter(&content)
                    .filter_map(|c| c.get(1).or(c.get(2)).map(|m| m.as_str().to_string()))
                    .filter(|n| !n.starts_with('_') && n != "defineStore" && n != store_name)
                    .collect();

                if !actions.is_empty() {
                    out.push_str("**Actions/Getters:**\n");
                    for a in &actions {
                        out.push_str(&format!("- `{}`\n", a));
                    }
                    out.push('\n');
                }
            }
        }

        if store_count == 0 {
            out.push_str("No Pinia stores found (`defineStore` not detected).\n");
        }

        Ok(out)
    }
}

/// Try to resolve a Vue import path to an actual file
fn resolve_vue_import(_root: &Path, candidate: &Path) -> Option<PathBuf> {
    // Direct match
    if candidate.exists() {
        return Some(candidate.to_path_buf());
    }
    // Try adding .vue
    let with_vue = candidate.with_extension("vue");
    if with_vue.exists() {
        return Some(with_vue);
    }
    // Try index.vue
    let index = candidate.join("index.vue");
    if index.exists() {
        return Some(index);
    }
    // Try .ts / .js
    for ext in ["ts", "js", "tsx", "jsx"] {
        let with_ext = candidate.with_extension(ext);
        if with_ext.exists() {
            return Some(with_ext);
        }
    }
    None
}
