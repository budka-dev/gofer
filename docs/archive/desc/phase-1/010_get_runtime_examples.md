# Feature: Get Runtime Examples

**Feature ID**: PHASE1-010  
**Priority**: 🔥🔥🔥🔥 CRITICAL  
**Estimated Effort**: 3 days  
**Phase**: 1 - Runtime & Evolution Context  
**Dependencies**: Test coverage (PHASE1-009), Index infrastructure (PHASE0-001)

---

## Overview

Runtime examples provide real-world usage examples extracted from test files, documentation, and actual code invocations. This helps developers understand how functions are actually used in practice, not just their API signatures.

### Problem Statement

Developers face these challenges when trying to use unfamiliar code:

1. **Unclear usage**: Function signature exists, but how to call it isn't obvious
2. **Missing context**: Don't know what typical input values look like
3. **Error handling**: Unsure how to handle errors properly
4. **Integration patterns**: Don't know common usage patterns
5. **Example scarcity**: Documentation examples are often outdated or missing

Example scenario:
```rust
// Developer sees this function
pub async fn connect_to_database(
    config: &DatabaseConfig,
    pool_size: usize,
) -> Result<Connection, DatabaseError> { ... }

// Questions:
// - What does a typical DatabaseConfig look like?
// - What's a reasonable pool_size?
// - How should I handle DatabaseError?
// - Are there retries? Timeouts?
```

With runtime examples, the developer can see:
```rust
// From tests/integration/db_test.rs:25
let config = DatabaseConfig {
    host: "localhost",
    port: 5432,
    database: "test_db",
    timeout: Duration::from_secs(30),
};
let conn = connect_to_database(&config, 10).await?;

// From src/api/server.rs:142
match connect_to_database(&prod_config, 50).await {
    Ok(conn) => info!("Connected to database"),
    Err(DatabaseError::ConnectionTimeout) => retry_connection(),
    Err(e) => panic!("Fatal database error: {}", e),
}
```

### Goals

1. ✅ **Extract real examples**: Find actual function calls from tests and production code
2. ✅ **Contextualize usage**: Show surrounding code, variable values, error handling
3. ✅ **Rank by relevance**: Prioritize simple, clear examples over complex ones
4. ✅ **Multiple sources**: Extract from unit tests, integration tests, docs, production code
5. ✅ **Language support**: Rust, TypeScript, Python, JavaScript
6. ✅ **Fast retrieval**: < 1s to find examples for a function
7. ✅ **Rich metadata**: Include file location, test context, success/failure info

### Non-Goals

1. ❌ **Dynamic execution**: Not running code or capturing runtime values
2. ❌ **Performance profiling**: Not measuring execution time or resource usage
3. ❌ **Code generation**: Not auto-generating examples
4. ❌ **Documentation generation**: Not creating API docs (just examples)
5. ❌ **Completeness**: Not exhaustive coverage of all possible usage patterns

---

## Architecture

### System Components

```
┌──────────────────────────────────────────────────────────────┐
│                  Runtime Examples System                      │
└──────────────────────────────────────────────────────────────┘
                           │
           ┌───────────────┴────────────────┐
           │                                │
    ┌──────▼──────┐                 ┌──────▼──────┐
    │   Example   │                 │   Example   │
    │  Extractor  │                 │   Ranker    │
    └──────┬──────┘                 └──────┬──────┘
           │                                │
    ┌──────┴──────┐                 ┌──────┴──────┐
    │             │                 │             │
┌───▼───┐   ┌────▼────┐      ┌─────▼─────┐ ┌─────▼─────┐
│ Test  │   │  Doc    │      │ Simplicity│ │ Context   │
│Finder │   │Extractor│      │  Scorer   │ │ Enricher  │
└───┬───┘   └────┬────┘      └─────┬─────┘ └─────┬─────┘
    │            │                 │             │
    └────────────┴─────────────────┴─────────────┘
                    │
            ┌───────▼────────┐
            │  SQLite Index  │
            │  - call_sites  │
            │  - references  │
            │  - symbols     │
            └────────────────┘
```

### Data Model

```sql
-- Store extracted examples
CREATE TABLE runtime_examples (
    id INTEGER PRIMARY KEY,
    symbol_id INTEGER NOT NULL,       -- Function being called
    call_site_file_id INTEGER NOT NULL,
    call_site_line INTEGER NOT NULL,
    example_type TEXT NOT NULL,       -- 'test', 'prod', 'doc', 'example'
    code_snippet TEXT NOT NULL,       -- The actual example code
    context_before TEXT,              -- 3 lines before
    context_after TEXT,               -- 3 lines after
    complexity_score REAL NOT NULL,   -- 0-1 (lower = simpler)
    relevance_score REAL NOT NULL,    -- 0-1 (higher = more relevant)
    has_error_handling BOOLEAN,
    has_comments BOOLEAN,
    extracted_at INTEGER NOT NULL,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
    FOREIGN KEY (call_site_file_id) REFERENCES files(id) ON DELETE CASCADE
);

CREATE INDEX idx_runtime_examples_symbol ON runtime_examples(symbol_id);
CREATE INDEX idx_runtime_examples_type ON runtime_examples(example_type);
CREATE INDEX idx_runtime_examples_relevance ON runtime_examples(relevance_score DESC);

-- Track variable assignments near call sites
CREATE TABLE example_context (
    id INTEGER PRIMARY KEY,
    example_id INTEGER NOT NULL,
    variable_name TEXT NOT NULL,
    variable_type TEXT,
    variable_value TEXT,              -- Literal value if available
    line_offset INTEGER NOT NULL,     -- Lines before/after call site
    FOREIGN KEY (example_id) REFERENCES runtime_examples(id) ON DELETE CASCADE
);

CREATE INDEX idx_example_context_example ON example_context(example_id);
```

### Example Extraction Strategy

**Four-stage process**:

1. **Call Site Discovery**
   - Use existing `references` table to find all call sites
   - Filter by reference type: `call`, `method_call`
   - Collect file locations and line numbers

2. **Code Extraction**
   - Read source file at call site
   - Extract call expression + surrounding context
   - Parse variables, literals, method chains
   - Capture error handling patterns (match, if let, ?)

3. **Context Enrichment**
   - Identify variable definitions near call site
   - Extract literal values, constants
   - Detect test assertions (assert!, assert_eq!)
   - Parse doc comments if in documentation

4. **Scoring & Ranking**
   - **Complexity scoring**: Fewer lines, fewer variables = simpler
   - **Relevance scoring**: Tests > docs > production, recent > old
   - **Quality scoring**: Has error handling, has comments, clear variable names

---

## API Specification

### Tool: `get_runtime_examples`

**Description**: Finds real-world usage examples for a function or method.

**Parameters**:

```typescript
interface GetRuntimeExamplesParams {
  // Target function
  symbol: string;                     // Function name
  file?: string;                      // File path to disambiguate
  
  // Filtering
  example_type?: string[];            // ['test', 'prod', 'doc'] - default: all
  max_examples?: number;              // Maximum results (default: 10)
  min_quality?: number;               // Minimum quality score 0-1 (default: 0.3)
  
  // Display options
  include_context?: boolean;          // Show surrounding code (default: true)
  include_variable_values?: boolean;  // Show variable assignments (default: true)
  show_complexity?: boolean;          // Show complexity metrics (default: false)
}
```

**Response**:

```typescript
interface RuntimeExamplesResponse {
  symbol: {
    name: string;
    file: string;
    line: number;
    signature: string;
  };
  examples: Example[];
  summary: {
    total_found: number;
    by_type: {
      test: number;
      prod: number;
      doc: number;
      example: number;
    };
  };
}

interface Example {
  id: string;
  type: 'test' | 'prod' | 'doc' | 'example';
  location: {
    file: string;
    line: number;
    function: string;                 // Containing function name
  };
  code: CodeExample;
  quality: QualityMetrics;
  context?: ContextInfo;
}

interface CodeExample {
  snippet: string;                    // The actual call expression
  context_before?: string[];          // Lines before call
  context_after?: string[];           // Lines after call
  full_function?: string;             // Entire containing function (if test)
}

interface QualityMetrics {
  relevance_score: number;            // 0-1
  complexity_score: number;           // 0-1 (lower is better)
  overall_quality: number;            // Weighted combination
  has_error_handling: boolean;
  has_comments: boolean;
  has_assertions: boolean;            // For tests
  line_count: number;
}

interface ContextInfo {
  variables: VariableContext[];
  constants: ConstantContext[];
  test_name?: string;                 // If from test
  test_description?: string;          // Parsed from test name
}

interface VariableContext {
  name: string;
  type?: string;
  value?: string;                     // Literal value if available
  defined_at: number;                 // Line offset from call site
}

interface ConstantContext {
  name: string;
  value: string;
}
```

**Example Usage**:

```typescript
// Find examples for a specific function
const examples = await get_runtime_examples({
  symbol: "connect_to_database",
  file: "src/storage/db.rs",
  example_type: ["test", "prod"],
  max_examples: 5
});

// Find simple examples only
const simpleExamples = await get_runtime_examples({
  symbol: "parse_json",
  min_quality: 0.7,
  include_context: false
});
```

---

## Implementation

### Component 1: Example Extractor

```rust
// src/examples/extractor.rs

use anyhow::Result;
use tree_sitter::{Parser, Query, QueryCursor, Node};
use crate::storage::{Store, models::Symbol};

pub struct ExampleExtractor {
    store: Store,
    rust_call_query: Query,
    typescript_call_query: Query,
}

impl ExampleExtractor {
    pub fn new(store: Store) -> Result<Self> {
        let rust_source = r#"
            ; Function calls
            (call_expression
              function: [
                (identifier) @fn_name
                (field_expression field: (field_identifier) @method_name)
                (scoped_identifier name: (identifier) @scoped_name)
              ]
              arguments: (arguments) @args) @call
            
            ; Method calls
            (call_expression
              function: (field_expression
                value: (_)
                field: (field_identifier) @method_name)
              arguments: (arguments) @args) @method_call
        "#;

        let typescript_source = r#"
            (call_expression
              function: [
                (identifier) @fn_name
                (member_expression property: (property_identifier) @method_name)
              ]
              arguments: (arguments) @args) @call
        "#;

        Ok(Self {
            store,
            rust_call_query: Query::new(&tree_sitter_rust::language(), rust_source)?,
            typescript_call_query: Query::new(
                &tree_sitter_typescript::language_typescript(),
                typescript_source
            )?,
        })
    }

    /// Extract all examples for a given symbol
    pub async fn extract_examples(&self, symbol_id: i64) -> Result<Vec<RawExample>> {
        // Get all call sites from references table
        let call_sites = self.get_call_sites(symbol_id).await?;
        
        let mut examples = Vec::new();
        for site in call_sites {
            if let Ok(example) = self.extract_single_example(&site).await {
                examples.push(example);
            }
        }

        Ok(examples)
    }

    async fn get_call_sites(&self, symbol_id: i64) -> Result<Vec<CallSite>> {
        let conn = self.store.conn().await?;
        
        let mut stmt = conn.prepare(
            "SELECT 
                r.file_id,
                r.line,
                r.reference_type,
                f.path
             FROM references r
             JOIN files f ON r.file_id = f.id
             WHERE r.target_symbol_id = ?
             AND r.reference_type IN ('call', 'method_call')
             ORDER BY f.path"
        )?;

        let sites = stmt.query_map([symbol_id], |row| {
            Ok(CallSite {
                file_id: row.get(0)?,
                line: row.get(1)?,
                reference_type: row.get(2)?,
                file_path: row.get(3)?,
            })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(sites)
    }

    async fn extract_single_example(&self, site: &CallSite) -> Result<RawExample> {
        // Read file content
        let content = tokio::fs::read(&site.file_path).await?;
        let text = String::from_utf8_lossy(&content);
        
        // Determine example type
        let example_type = self.classify_example_type(&site.file_path);
        
        // Parse and find the call expression
        let lang = Language::from_path(&site.file_path);
        let mut parser = Parser::new();
        parser.set_language(&lang.tree_sitter_lang())?;
        
        let tree = parser.parse(&content, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse"))?;
        
        // Find the node at the specific line
        let call_node = self.find_node_at_line(&tree, site.line)?;
        
        // Extract the call expression
        let snippet = call_node.utf8_text(&content)?.to_string();
        
        // Extract context (3 lines before/after)
        let (context_before, context_after) = self.extract_context(&text, site.line)?;
        
        // Find containing function
        let containing_fn = self.find_containing_function(&tree, call_node, &content)?;
        
        // Extract variable context
        let variables = self.extract_variable_context(&tree, call_node, &content)?;
        
        // Detect patterns
        let has_error_handling = self.detect_error_handling(&tree, call_node, &content);
        let has_comments = self.has_nearby_comments(&text, site.line);
        
        Ok(RawExample {
            file_path: site.file_path.clone(),
            line: site.line,
            example_type,
            snippet,
            context_before,
            context_after,
            containing_function: containing_fn,
            variables,
            has_error_handling,
            has_comments,
        })
    }

    fn classify_example_type(&self, file_path: &str) -> ExampleType {
        if file_path.contains("/tests/") || file_path.ends_with("_test.rs") {
            ExampleType::Test
        } else if file_path.contains("/examples/") || file_path.ends_with("_example.rs") {
            ExampleType::Example
        } else if file_path.ends_with(".md") || file_path.contains("/docs/") {
            ExampleType::Doc
        } else {
            ExampleType::Prod
        }
    }

    fn extract_context(&self, text: &str, target_line: usize) -> Result<(Vec<String>, Vec<String>)> {
        let lines: Vec<&str> = text.lines().collect();
        let idx = target_line.saturating_sub(1); // 0-indexed
        
        let before_start = idx.saturating_sub(3);
        let before = lines[before_start..idx]
            .iter()
            .map(|s| s.to_string())
            .collect();
        
        let after_end = (idx + 4).min(lines.len());
        let after = lines[(idx + 1)..after_end]
            .iter()
            .map(|s| s.to_string())
            .collect();
        
        Ok((before, after))
    }

    fn find_containing_function(
        &self,
        tree: &tree_sitter::Tree,
        node: Node,
        content: &[u8],
    ) -> Result<Option<String>> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "function_item" || parent.kind() == "function_declaration" {
                // Extract function name
                let name_node = parent.child_by_field_name("name")
                    .ok_or_else(|| anyhow::anyhow!("No function name"))?;
                return Ok(Some(name_node.utf8_text(content)?.to_string()));
            }
            current = parent;
        }
        Ok(None)
    }

    fn extract_variable_context(
        &self,
        tree: &tree_sitter::Tree,
        call_node: Node,
        content: &[u8],
    ) -> Result<Vec<VariableContext>> {
        let mut variables = Vec::new();
        
        // Find arguments to the function call
        if let Some(args_node) = call_node.child_by_field_name("arguments") {
            let mut cursor = args_node.walk();
            for child in args_node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let var_name = child.utf8_text(content)?;
                    
                    // Try to find variable definition
                    if let Some(def) = self.find_variable_definition(tree, call_node, var_name, content) {
                        variables.push(def);
                    }
                } else if is_literal(&child) {
                    // Inline literal value
                    variables.push(VariableContext {
                        name: "_literal".into(),
                        var_type: Some(child.kind().to_string()),
                        value: Some(child.utf8_text(content)?.to_string()),
                        defined_at: 0,
                    });
                }
            }
        }
        
        Ok(variables)
    }

    fn find_variable_definition(
        &self,
        tree: &tree_sitter::Tree,
        from_node: Node,
        var_name: &str,
        content: &[u8],
    ) -> Option<VariableContext> {
        // Walk upward to find let bindings
        let mut current = from_node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "let_declaration" || parent.kind() == "variable_declaration" {
                if let Some(pattern) = parent.child_by_field_name("pattern") {
                    if pattern.utf8_text(content).ok()? == var_name {
                        // Found the definition
                        let value_node = parent.child_by_field_name("value")?;
                        let value_text = value_node.utf8_text(content).ok()?.to_string();
                        
                        let line_offset = (parent.start_position().row as i32) 
                            - (from_node.start_position().row as i32);
                        
                        return Some(VariableContext {
                            name: var_name.to_string(),
                            var_type: None, // Could parse type annotation
                            value: Some(value_text),
                            defined_at: line_offset,
                        });
                    }
                }
            }
            current = parent;
        }
        None
    }

    fn detect_error_handling(&self, tree: &tree_sitter::Tree, call_node: Node, content: &[u8]) -> bool {
        // Check if call is inside match, if let, try block, or uses ?
        let mut current = call_node;
        while let Some(parent) = current.parent() {
            match parent.kind() {
                "match_expression" | "if_let_expression" | "try_expression" => return true,
                _ => {}
            }
            current = parent;
        }
        
        // Check for ? operator
        if let Some(sibling) = call_node.next_sibling() {
            if sibling.kind() == "?" {
                return true;
            }
        }
        
        false
    }

    fn has_nearby_comments(&self, text: &str, line: usize) -> bool {
        let lines: Vec<&str> = text.lines().collect();
        let idx = line.saturating_sub(1);
        
        // Check 2 lines before
        for i in idx.saturating_sub(2)..idx {
            if lines.get(i).map_or(false, |l| l.trim().starts_with("//")) {
                return true;
            }
        }
        false
    }

    fn find_node_at_line(&self, tree: &tree_sitter::Tree, line: usize) -> Result<Node> {
        let root = tree.root_node();
        let target_row = line - 1; // 0-indexed
        
        self.find_node_at_row_recursive(root, target_row)
            .ok_or_else(|| anyhow::anyhow!("No node found at line {}", line))
    }

    fn find_node_at_row_recursive(&self, node: Node, target_row: usize) -> Option<Node> {
        if node.start_position().row <= target_row && node.end_position().row >= target_row {
            // This node contains the target line
            
            // Check children for a more specific match
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(found) = self.find_node_at_row_recursive(child, target_row) {
                    return Some(found);
                }
            }
            
            // No child was more specific, return this node
            return Some(node);
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct RawExample {
    pub file_path: String,
    pub line: usize,
    pub example_type: ExampleType,
    pub snippet: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
    pub containing_function: Option<String>,
    pub variables: Vec<VariableContext>,
    pub has_error_handling: bool,
    pub has_comments: bool,
}

#[derive(Debug, Clone)]
pub struct CallSite {
    pub file_id: i64,
    pub line: usize,
    pub reference_type: String,
    pub file_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExampleType {
    Test,
    Prod,
    Doc,
    Example,
}

#[derive(Debug, Clone)]
pub struct VariableContext {
    pub name: String,
    pub var_type: Option<String>,
    pub value: Option<String>,
    pub defined_at: i32, // Line offset from call site (negative = before)
}

fn is_literal(node: &Node) -> bool {
    matches!(
        node.kind(),
        "string_literal" | "integer_literal" | "float_literal" | "boolean_literal"
    )
}
```

### Component 2: Example Ranker

```rust
// src/examples/ranker.rs

use anyhow::Result;
use super::extractor::RawExample;

pub struct ExampleRanker {
    weights: ScoringWeights,
}

#[derive(Debug, Clone)]
pub struct ScoringWeights {
    pub simplicity: f64,       // 0.4 - prefer simple examples
    pub recency: f64,           // 0.2 - prefer recent code
    pub source_type: f64,       // 0.3 - tests > examples > docs > prod
    pub quality_features: f64,  // 0.1 - comments, error handling
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            simplicity: 0.4,
            recency: 0.2,
            source_type: 0.3,
            quality_features: 0.1,
        }
    }
}

impl ExampleRanker {
    pub fn new() -> Self {
        Self {
            weights: ScoringWeights::default(),
        }
    }

    /// Score and rank examples
    pub fn rank_examples(&self, examples: &mut [RawExample]) {
        for example in examples.iter_mut() {
            let complexity = self.calculate_complexity(example);
            let relevance = self.calculate_relevance(example);
            
            // Store scores (would need to add fields to RawExample)
            // example.complexity_score = complexity;
            // example.relevance_score = relevance;
        }
        
        // Sort by overall quality (could add field to store this)
        examples.sort_by(|a, b| {
            let score_a = self.overall_quality(a);
            let score_b = self.overall_quality(b);
            score_b.partial_cmp(&score_a).unwrap()
        });
    }

    fn calculate_complexity(&self, example: &RawExample) -> f64 {
        let mut score = 0.0;
        
        // Line count (fewer is better)
        let line_count = example.snippet.lines().count();
        let line_penalty = (line_count as f64 / 20.0).min(1.0); // Normalize to 0-1
        score += line_penalty * 0.4;
        
        // Number of variables (fewer is better)
        let var_penalty = (example.variables.len() as f64 / 10.0).min(1.0);
        score += var_penalty * 0.3;
        
        // Nesting depth (less is better)
        let nesting = self.estimate_nesting_depth(&example.snippet);
        let nesting_penalty = (nesting as f64 / 5.0).min(1.0);
        score += nesting_penalty * 0.3;
        
        score // 0.0 = simple, 1.0 = complex
    }

    fn calculate_relevance(&self, example: &RawExample) -> f64 {
        let mut score = 0.0;
        
        // Source type scoring
        let type_score = match example.example_type {
            ExampleType::Test => 1.0,       // Best: clear, focused
            ExampleType::Example => 0.9,    // Very good: designed to teach
            ExampleType::Doc => 0.7,        // Good: explanatory
            ExampleType::Prod => 0.5,       // OK: real-world but complex
        };
        score += type_score * self.weights.source_type;
        
        // Quality features
        let mut quality = 0.0;
        if example.has_error_handling {
            quality += 0.5;
        }
        if example.has_comments {
            quality += 0.3;
        }
        if example.containing_function.is_some() {
            quality += 0.2;
        }
        score += quality * self.weights.quality_features;
        
        // Simplicity (inverted complexity)
        let complexity = self.calculate_complexity(example);
        score += (1.0 - complexity) * self.weights.simplicity;
        
        // Recency (would need file modification time)
        // score += recency_score * self.weights.recency;
        
        score.min(1.0)
    }

    fn overall_quality(&self, example: &RawExample) -> f64 {
        let complexity = self.calculate_complexity(example);
        let relevance = self.calculate_relevance(example);
        
        // Weighted average
        relevance * 0.7 + (1.0 - complexity) * 0.3
    }

    fn estimate_nesting_depth(&self, code: &str) -> usize {
        let mut max_depth = 0;
        let mut current_depth = 0;
        
        for ch in code.chars() {
            match ch {
                '{' | '(' | '[' => {
                    current_depth += 1;
                    max_depth = max_depth.max(current_depth);
                }
                '}' | ')' | ']' => {
                    current_depth = current_depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        
        max_depth
    }
}
```

### Component 3: MCP Tool Handler

```rust
// src/tools/runtime_examples.rs

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::examples::{ExampleExtractor, ExampleRanker};
use crate::storage::Store;

#[derive(Debug, Deserialize)]
pub struct GetRuntimeExamplesParams {
    pub symbol: String,
    pub file: Option<String>,
    pub example_type: Option<Vec<String>>,
    pub max_examples: Option<usize>,
    pub min_quality: Option<f64>,
    pub include_context: Option<bool>,
    pub include_variable_values: Option<bool>,
    pub show_complexity: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeExamplesResponse {
    pub symbol: SymbolInfo,
    pub examples: Vec<Example>,
    pub summary: ExampleSummary,
}

#[derive(Debug, Serialize)]
pub struct SymbolInfo {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct Example {
    pub id: String,
    pub r#type: String,
    pub location: Location,
    pub code: CodeExample,
    pub quality: QualityMetrics,
    pub context: Option<ContextInfo>,
}

#[derive(Debug, Serialize)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub function: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CodeExample {
    pub snippet: String,
    pub context_before: Option<Vec<String>>,
    pub context_after: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct QualityMetrics {
    pub relevance_score: f64,
    pub complexity_score: f64,
    pub overall_quality: f64,
    pub has_error_handling: bool,
    pub has_comments: bool,
    pub line_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ContextInfo {
    pub variables: Vec<VariableInfo>,
    pub test_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VariableInfo {
    pub name: String,
    pub r#type: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExampleSummary {
    pub total_found: usize,
    pub by_type: TypeBreakdown,
}

#[derive(Debug, Serialize)]
pub struct TypeBreakdown {
    pub test: usize,
    pub prod: usize,
    pub doc: usize,
    pub example: usize,
}

pub async fn get_runtime_examples(
    params: GetRuntimeExamplesParams,
    store: Store,
) -> Result<RuntimeExamplesResponse> {
    // Find symbol
    let symbol = store.find_symbol(&params.symbol, params.file.as_deref()).await?
        .ok_or_else(|| anyhow::anyhow!("Symbol not found: {}", params.symbol))?;
    
    // Extract examples
    let extractor = ExampleExtractor::new(store.clone())?;
    let mut raw_examples = extractor.extract_examples(symbol.id).await?;
    
    // Filter by type if specified
    if let Some(types) = &params.example_type {
        raw_examples.retain(|ex| {
            let type_str = format!("{:?}", ex.example_type).to_lowercase();
            types.iter().any(|t| t.to_lowercase() == type_str)
        });
    }
    
    // Rank examples
    let ranker = ExampleRanker::new();
    ranker.rank_examples(&mut raw_examples);
    
    // Apply quality filter
    let min_quality = params.min_quality.unwrap_or(0.3);
    raw_examples.retain(|ex| ranker.overall_quality(ex) >= min_quality);
    
    // Limit results
    let max_examples = params.max_examples.unwrap_or(10);
    raw_examples.truncate(max_examples);
    
    // Convert to response format
    let include_context = params.include_context.unwrap_or(true);
    let include_vars = params.include_variable_values.unwrap_or(true);
    
    let examples: Vec<Example> = raw_examples.iter().enumerate()
        .map(|(idx, ex)| convert_to_example(ex, idx, include_context, include_vars, &ranker))
        .collect();
    
    // Summary
    let summary = calculate_summary(&raw_examples);
    
    Ok(RuntimeExamplesResponse {
        symbol: SymbolInfo {
            name: symbol.name.clone(),
            file: symbol.file_path.clone(),
            line: symbol.start_line,
            signature: symbol.signature.unwrap_or_default(),
        },
        examples,
        summary,
    })
}

fn convert_to_example(
    raw: &RawExample,
    idx: usize,
    include_context: bool,
    include_vars: bool,
    ranker: &ExampleRanker,
) -> Example {
    let complexity = ranker.calculate_complexity(raw);
    let relevance = ranker.calculate_relevance(raw);
    let overall = ranker.overall_quality(raw);
    
    Example {
        id: format!("ex-{}", idx),
        r#type: format!("{:?}", raw.example_type).to_lowercase(),
        location: Location {
            file: raw.file_path.clone(),
            line: raw.line,
            function: raw.containing_function.clone(),
        },
        code: CodeExample {
            snippet: raw.snippet.clone(),
            context_before: if include_context {
                Some(raw.context_before.clone())
            } else {
                None
            },
            context_after: if include_context {
                Some(raw.context_after.clone())
            } else {
                None
            },
        },
        quality: QualityMetrics {
            relevance_score: relevance,
            complexity_score: complexity,
            overall_quality: overall,
            has_error_handling: raw.has_error_handling,
            has_comments: raw.has_comments,
            line_count: raw.snippet.lines().count(),
        },
        context: if include_vars && !raw.variables.is_empty() {
            Some(ContextInfo {
                variables: raw.variables.iter().map(|v| VariableInfo {
                    name: v.name.clone(),
                    r#type: v.var_type.clone(),
                    value: v.value.clone(),
                }).collect(),
                test_name: raw.containing_function.clone(),
            })
        } else {
            None
        },
    }
}

fn calculate_summary(examples: &[RawExample]) -> ExampleSummary {
    let mut by_type = TypeBreakdown {
        test: 0,
        prod: 0,
        doc: 0,
        example: 0,
    };
    
    for ex in examples {
        match ex.example_type {
            ExampleType::Test => by_type.test += 1,
            ExampleType::Prod => by_type.prod += 1,
            ExampleType::Doc => by_type.doc += 1,
            ExampleType::Example => by_type.example += 1,
        }
    }
    
    ExampleSummary {
        total_found: examples.len(),
        by_type,
    }
}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_example_type() {
        let extractor = ExampleExtractor::new(store).unwrap();
        
        assert_eq!(
            extractor.classify_example_type("tests/api_test.rs"),
            ExampleType::Test
        );
        assert_eq!(
            extractor.classify_example_type("src/lib.rs"),
            ExampleType::Prod
        );
        assert_eq!(
            extractor.classify_example_type("examples/tutorial.rs"),
            ExampleType::Example
        );
    }

    #[test]
    fn test_complexity_scoring() {
        let ranker = ExampleRanker::new();
        
        let simple = RawExample {
            snippet: "add(2, 3)".into(),
            variables: vec![],
            ..Default::default()
        };
        
        let complex = RawExample {
            snippet: "match db.query() {\n  Ok(r) => process(r),\n  Err(e) => handle(e)\n}".into(),
            variables: vec![var1, var2, var3],
            ..Default::default()
        };
        
        assert!(ranker.calculate_complexity(&simple) < ranker.calculate_complexity(&complex));
    }
}
```

---

## Success Metrics

- **Extraction accuracy**: > 95% of call sites found
- **Relevance**: Top 3 examples rated "helpful" by developers > 80% of time
- **Performance**: < 1s to find examples
- **Coverage**: Examples found for > 90% of public functions

---

## Usage Examples

```bash
# Find examples for a function
mcp query get_runtime_examples '{"symbol": "parse_config", "max_examples": 5}'

# Test examples only
mcp query get_runtime_examples '{
  "symbol": "authenticate_user",
  "example_type": ["test"],
  "include_context": true
}'
```

**Response**:
```json
{
  "symbol": {
    "name": "authenticate_user",
    "file": "src/auth.rs",
    "line": 42,
    "signature": "pub fn authenticate_user(username: &str, password: &str) -> Result<User>"
  },
  "examples": [
    {
      "id": "ex-0",
      "type": "test",
      "location": {
        "file": "tests/auth_test.rs",
        "line": 15,
        "function": "test_successful_auth"
      },
      "code": {
        "snippet": "authenticate_user(\"alice\", \"secret123\")?",
        "context_before": [
          "let user = authenticate_user(\"alice\", \"secret123\")?;",
          "assert_eq!(user.username, \"alice\");"
        ]
      },
      "quality": {
        "relevance_score": 0.95,
        "complexity_score": 0.1,
        "overall_quality": 0.92,
        "has_error_handling": true,
        "has_comments": false,
        "line_count": 1
      },
      "context": {
        "variables": [
          {"name": "username", "value": "\"alice\""},
          {"name": "password", "value": "\"secret123\""}
        ],
        "test_name": "test_successful_auth"
      }
    }
  ],
  "summary": {
    "total_found": 8,
    "by_type": {"test": 5, "prod": 3, "doc": 0, "example": 0}
  }
}
```
