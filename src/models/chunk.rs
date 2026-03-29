use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

/// Represents an indexed file in the database
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IndexedFile {
    pub id: i64,
    pub path: String,
    pub last_modified: i64,
    pub content_hash: String,
}

/// Represents a code symbol (function, struct, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Symbol {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    #[serde(rename = "kind")]
    #[sqlx(rename = "kind")]
    pub kind: SymbolKind,
    pub line_start: i32,
    pub line_end: i32,
    pub signature: Option<String>,
}

/// Symbol kind enumeration
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[archive(check_bytes)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    #[serde(rename = "function")]
    Function,
    #[serde(rename = "struct")]
    Struct,
    #[serde(rename = "enum")]
    Enum,
    #[serde(rename = "impl")]
    Impl,
    #[serde(rename = "trait")]
    Trait,
    #[serde(rename = "interface")]
    Interface,
    #[serde(rename = "const")]
    Const,
    #[serde(rename = "type")]
    Type,
    #[serde(rename = "type_alias")]
    TypeAlias,
    #[serde(rename = "module")]
    Module,
    #[serde(rename = "class")]
    Class,
    #[serde(rename = "method")]
    Method,
    #[serde(rename = "local_var")]
    LocalVar,
}

impl SymbolKind {
    /// Parse from string (for SQLite compatibility)
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "function" | "fn" => SymbolKind::Function,
            "struct" => SymbolKind::Struct,
            "enum" => SymbolKind::Enum,
            "impl" => SymbolKind::Impl,
            "trait" => SymbolKind::Trait,
            "interface" => SymbolKind::Interface,
            "const" => SymbolKind::Const,
            "type" => SymbolKind::Type,
            "type_alias" => SymbolKind::TypeAlias,
            "module" | "mod" => SymbolKind::Module,
            "class" => SymbolKind::Class,
            "method" => SymbolKind::Method,
            "local_var" => SymbolKind::LocalVar,
            _ => SymbolKind::Function, // Default fallback
        }
    }

    /// Convert to string (for SQLite storage)
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Impl => "impl",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Const => "const",
            SymbolKind::Type => "type",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Module => "module",
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            SymbolKind::LocalVar => "local_var",
        }
    }
}

// Implement sqlx::Type for SymbolKind to work with SQLite TEXT fields
impl sqlx::Type<sqlx::Sqlite> for SymbolKind {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <String as sqlx::Type<sqlx::Sqlite>>::type_info()
    }
}

// Implement sqlx::Decode for reading from database
impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for SymbolKind {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <&str as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
        Ok(SymbolKind::from_str(s))
    }
}

// Implement sqlx::Encode for writing to database
impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for SymbolKind {
    fn encode_by_ref(
        &self,
        args: &mut Vec<sqlx::sqlite::SqliteArgumentValue<'q>>,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        args.push(sqlx::sqlite::SqliteArgumentValue::Text(
            std::borrow::Cow::Borrowed(self.as_str()),
        ));
        Ok(sqlx::encode::IsNull::No)
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A code chunk with its embedding vector (for LanceDB)
#[derive(Debug, Clone)]
pub struct CodeChunk {
    pub id: String,
    pub file_path: String,
    pub content: String,
    pub line_start: u32,
    pub line_end: u32,
    pub symbol_name: Option<String>,
    pub symbol_kind: Option<SymbolKind>,
    /// Путь к символу, например "UserService::save" или "mod auth -> fn login"
    pub symbol_path: Option<String>,
    /// Стек скоупов для контекст-инъекции в oversized-чанках
    #[allow(dead_code)]
    pub scopes: Vec<String>,
}

/// Search result combining vector and FTS results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SearchResult {
    pub file_path: String,
    pub content: String,
    pub line_start: u32,
    pub line_end: u32,
    pub score: f32,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum MatchType {
    Semantic,
    Keyword,
    Hybrid,
}

/// Symbol reference (dependency graph edge)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SymbolReference {
    pub id: i64,
    pub source_symbol_id: i64,
    pub target_name: String,
    pub target_symbol_id: Option<i64>,
    pub kind: String,
    pub line: i32,
}

/// Reference kind enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ReferenceKind {
    Call,      // Function/method call
    Import,    // Use/import statement
    TypeUsage, // Type annotation
    Inherit,   // Impl for, extends, implements
}

/// Import/dependency information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub path: String,       // "./components/Button" or "lodash"
    pub items: Vec<String>, // ["Button", "ButtonProps"] or ["default"]
    pub is_relative: bool,  // true for local imports
    pub line: u32,
}

/// Bundled context for LLM consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    pub main_file: String,
    pub main_content: String,
    pub dependencies: Vec<DependencyFile>,
    pub markdown: String,
    pub total_lines: usize,
    pub total_tokens_estimate: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyFile {
    pub path: String,
    pub content: String,
    pub reason: String, // "imported type", "imported component", etc.
    pub depth: u32,
}

/// Project dependency (from Cargo.toml or package.json)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Dependency {
    pub id: i64,
    pub name: String,
    pub version: String,
    pub ecosystem: String, // "cargo" or "npm"
    pub features: Option<String>,
    pub dev_only: i32,
    pub updated_at: i64,
}

/// Project rule/best practice
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Rule {
    pub id: i64,
    pub category: String,
    pub rule: String,
    pub priority: i32,
    pub source: Option<String>,
}

/// Golden sample file marker
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[allow(dead_code)]
pub struct GoldenSample {
    pub id: i64,
    pub file_id: i64,
    pub category: Option<String>,
    pub description: Option<String>,
}

/// Project context for LLM injection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProjectContext {
    pub dependencies: Vec<Dependency>,
    pub rules: Vec<Rule>,
    pub golden_samples: Vec<String>,
    pub prompt_fragment: String,
}

/// Dependency usage record - tracks where a dependency is imported
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DependencyUsage {
    pub id: i64,
    pub dependency_id: i64,
    pub file_id: i64,
    pub line: i32,
    pub usage_type: String,
    pub import_path: String,
    pub items: Option<String>,
}

/// Impact analysis result - files affected by a dependency
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DependencyImpact {
    pub dependency_name: String,
    pub dependency_version: String,
    pub files: Vec<ImpactedFile>,
    pub total_usages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ImpactedFile {
    pub path: String,
    pub lines: Vec<i32>,
    pub usage_types: Vec<String>,
}

/// Active compiler error
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ActiveError {
    pub id: i64,
    pub file_path: String,
    pub line: i32,
    pub column: Option<i32>,
    pub severity: String,
    pub code: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
    pub updated_at: i64,
}

/// Config key from .env.example or config structs
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConfigKey {
    pub id: i64,
    pub key_name: String,
    pub data_type: Option<String>,
    pub source: String,
    pub description: Option<String>,
    pub default_value: Option<String>,
    pub required: i32,
}

/// Vue component tree
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct VueTree {
    pub id: i64,
    pub file_id: i64,
    pub tree_text: String,
    pub components: String,
    pub updated_at: i64,
}

// === MCP Support Types ===

/// Symbol with file path (for MCP tools)
#[derive(
    Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[archive(check_bytes)]
pub struct SymbolWithPath {
    pub id: i64,
    pub name: String,
    #[serde(rename = "kind")]
    #[sqlx(rename = "kind")]
    pub kind: SymbolKind,
    pub line: i32,
    pub end_line: i32,
    pub signature: Option<String>,
    pub file_path: String,
}

/// Reference with file path (for MCP tools)
#[derive(
    Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[archive(check_bytes)]
pub struct ReferenceWithPath {
    pub id: i64,
    pub target_name: String,
    pub ref_kind: String,
    pub line: i32,
    pub file_path: String,
}

/// Dependency usage info with file path (for MCP tools)
#[derive(
    Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[archive(check_bytes)]
pub struct DependencyUsageInfo {
    pub id: i64,
    pub line: i32,
    pub usage_type: String,
    pub import_path: String,
    pub items: Option<String>,
    pub file_path: String,
}

/// API endpoint info (for MCP tools)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApiEndpointInfo {
    pub id: i64,
    pub method: String,
    pub path: String,
    pub file_id: i64,
    pub line: Option<i32>,
}

/// Frontend API call info (for MCP tools)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FrontendApiCallInfo {
    pub id: i64,
    pub method: Option<String>,
    pub path: String,
    pub path_pattern: Option<String>,
    pub file_id: i64,
    pub line: Option<i32>,
}

// === Structural Fingerprinting Types ===

/// Type fingerprint для Jaccard-сравнения полей
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TypeFingerprint {
    pub id: i64,
    pub file_id: i64,
    pub symbol_id: i64,
    pub type_name: String,
    pub language: String,
    pub fields_json: String,
    pub fields_normalized: String,
    pub field_count: i32,
    pub file_path: String,
}

/// Cross-stack link между бэкенд и фронтенд сущностями
#[derive(
    Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[archive(check_bytes)]
pub struct CrossStackLink {
    pub id: i64,
    pub source_file: String,
    pub target_file: String,
    pub source_symbol: String,
    pub target_symbol: String,
    pub link_type: String,
    pub weight: f64,
    pub metadata: Option<String>,
    pub created_at: i64,
}

/// Извлечённое поле из struct/interface (для fingerprint-сбора)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TypeField {
    pub name: String,
    pub field_type: Option<String>,
    pub normalized: String, // lower, no separators: "userid" <- "user_id" / "userId"
}
