# Feature: Find Error Patterns

**Feature ID**: PHASE1-011  
**Priority**: 🔥🔥🔥 HIGH  
**Estimated Effort**: 4 days  
**Phase**: 1 - Runtime & Evolution Context  
**Dependencies**: Runtime examples (PHASE1-010), Test coverage (PHASE1-009)

---

## Overview

Find error patterns analyzes where and how code fails by extracting error handling patterns, panic sites, Result propagation chains, and common failure scenarios from tests and production code.

### Problem Statement

Developers need to understand:

1. **Where code can fail**: Which functions return errors? What error types?
2. **Common failure modes**: What are the typical ways this code breaks?
3. **Error handling patterns**: How should errors be handled in this codebase?
4. **Panic risks**: Where are unwrap(), expect(), panic!() calls?
5. **Recovery strategies**: How does the code recover from errors?

Example scenario:
```rust
// Developer wants to call this function
pub async fn fetch_user_data(user_id: i64) -> Result<UserData, ApiError> {
    // What errors can this return?
    // - ApiError::NotFound?
    // - ApiError::NetworkError?
    // - ApiError::Unauthorized?
    // How should I handle each error type?
}
```

With error pattern analysis:
```rust
// Common error patterns found:
// 1. ApiError::NotFound (40% of failures in tests)
//    - Usually handled by returning default or prompting user
// 2. ApiError::NetworkError (30% of failures)
//    - Usually retried with exponential backoff
// 3. ApiError::Unauthorized (20% of failures)
//    - Usually triggers re-authentication flow
// 4. ApiError::RateLimited (10% of failures)
//    - Usually waits and retries

// Panic risks:
// - Line 156: user_data.profile.unwrap() - panics if profile is None
// - Line 203: .expect("Cache must exist") - panics if cache unavailable
```

### Goals

1. ✅ **Error site detection**: Find all error returns, panics, unwraps
2. ✅ **Error type analysis**: Catalog error types and their variants
3. ✅ **Pattern extraction**: Identify common error handling patterns
4. ✅ **Failure frequency**: Rank errors by how often they occur in tests
5. ✅ **Propagation tracking**: Trace error propagation through call chains
6. ✅ **Recovery strategies**: Document how errors are handled/recovered
7. ✅ **Panic audit**: Flag dangerous unwrap/expect calls

### Non-Goals

1. ❌ **Runtime instrumentation**: Not monitoring production errors
2. ❌ **Error rate tracking**: Not measuring actual error rates
3. ❌ **Log analysis**: Not parsing application logs
4. ❌ **Exception tracking**: Not integrating with Sentry/etc
5. ❌ **Auto-fixing**: Not automatically removing unwraps

---

## Architecture

### System Components

```
┌───────────────────────────────────────────────────────────┐
│              Error Pattern Analysis System                 │
└───────────────────────────────────────────────────────────┘
                         │
         ┌───────────────┴──────────────┐
         │                              │
  ┌──────▼──────┐              ┌────────▼────────┐
  │   Error     │              │  Pattern        │
  │  Detector   │              │  Analyzer       │
  └──────┬──────┘              └────────┬────────┘
         │                              │
   ┌─────┴─────┐                  ┌─────┴─────┐
   │           │                  │           │
┌──▼──┐  ┌────▼────┐       ┌─────▼─────┐ ┌───▼───┐
│Panic│  │ Result  │       │ Frequency │ │Recov- │
│Scan │  │ Tracker │       │  Counter  │ │ ery   │
└──┬──┘  └────┬────┘       └─────┬─────┘ └───┬───┘
   │          │                  │           │
   └──────────┴──────────────────┴───────────┘
                     │
             ┌───────▼────────┐
             │  SQLite Index  │
             │  - error_sites │
             │  - symbols     │
             │  - references  │
             └────────────────┘
```

### Data Model

```sql
-- Track all error sites
CREATE TABLE error_sites (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    symbol_id INTEGER,                -- Function containing the error
    line INTEGER NOT NULL,
    error_type TEXT NOT NULL,         -- 'result', 'panic', 'unwrap', 'expect'
    error_enum TEXT,                  -- e.g., "ApiError", "io::Error"
    error_variant TEXT,               -- e.g., "NotFound", "PermissionDenied"
    code_snippet TEXT NOT NULL,
    severity TEXT NOT NULL,           -- 'low', 'medium', 'high', 'critical'
    is_test BOOLEAN DEFAULT FALSE,
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE SET NULL
);

CREATE INDEX idx_error_sites_file ON error_sites(file_id);
CREATE INDEX idx_error_sites_symbol ON error_sites(symbol_id);
CREATE INDEX idx_error_sites_type ON error_sites(error_type);
CREATE INDEX idx_error_sites_severity ON error_sites(severity);

-- Track error handling patterns
CREATE TABLE error_patterns (
    id INTEGER PRIMARY KEY,
    error_enum TEXT NOT NULL,
    error_variant TEXT,
    handling_pattern TEXT NOT NULL,   -- 'retry', 'fallback', 'propagate', 'log', 'ignore'
    code_example TEXT NOT NULL,
    file_id INTEGER NOT NULL,
    line INTEGER NOT NULL,
    frequency INTEGER DEFAULT 1,      -- How many times this pattern appears
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
);

CREATE INDEX idx_error_patterns_enum ON error_patterns(error_enum, error_variant);
CREATE INDEX idx_error_patterns_frequency ON error_patterns(frequency DESC);

-- Track panic/unwrap risks
CREATE TABLE panic_sites (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    symbol_id INTEGER,
    line INTEGER NOT NULL,
    panic_type TEXT NOT NULL,         -- 'panic!', 'unwrap', 'expect', 'unreachable!'
    message TEXT,                     -- expect() message or panic! text
    code_snippet TEXT NOT NULL,
    risk_level TEXT NOT NULL,         -- 'low', 'medium', 'high'
    has_guard BOOLEAN DEFAULT FALSE,  -- Has if/match guard before panic
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE SET NULL
);

CREATE INDEX idx_panic_sites_file ON panic_sites(file_id);
CREATE INDEX idx_panic_sites_risk ON panic_sites(risk_level);

-- Track error propagation chains
CREATE TABLE error_propagations (
    id INTEGER PRIMARY KEY,
    from_symbol_id INTEGER NOT NULL,
    to_symbol_id INTEGER NOT NULL,
    error_enum TEXT NOT NULL,
    propagation_method TEXT NOT NULL, -- '?', 'map_err', 'match'
    transforms_error BOOLEAN DEFAULT FALSE,
    FOREIGN KEY (from_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
    FOREIGN KEY (to_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
);

CREATE INDEX idx_error_propagations_from ON error_propagations(from_symbol_id);
CREATE INDEX idx_error_propagations_to ON error_propagations(to_symbol_id);
```

### Error Detection Strategy

**Multi-stage analysis**:

1. **Static Analysis** (ErrorDetector)
   - Parse all source files with tree-sitter
   - Find: `Result<T, E>` returns, `panic!()`, `unwrap()`, `expect()`
   - Extract error enum types from Result<T, E>
   - Identify error handling: match, if let, ?, map_err

2. **Pattern Recognition** (PatternAnalyzer)
   - Group errors by type and variant
   - Classify handling strategies: retry, fallback, propagate, log
   - Count pattern frequencies
   - Extract representative examples

3. **Risk Assessment** (RiskScorer)
   - Score panic sites by context:
     - High: unwrap() in prod code, no guard
     - Medium: expect() with unclear message
     - Low: unwrap() in test, or after explicit check
   - Identify missing error handling
   - Flag error swallowing (let _ = ...)

4. **Propagation Tracking** (PropagationTracer)
   - Trace error flows through call chains
   - Identify error transformation points (map_err, context)
   - Build error propagation graph

---

## API Specification

### Tool: `find_error_patterns`

**Description**: Analyzes error handling patterns, panic risks, and common failure modes.

**Parameters**:

```typescript
interface FindErrorPatternsParams {
  // Scope
  file?: string;                      // Analyze specific file
  symbol?: string;                    // Analyze specific function
  project?: boolean;                  // Analyze entire project
  
  // Filters
  error_type?: string[];              // ['panic', 'result', 'unwrap'] - default: all
  min_severity?: string;              // 'low' | 'medium' | 'high' | 'critical'
  include_tests?: boolean;            // Include test files (default: true)
  
  // Display options
  show_examples?: boolean;            // Include code examples (default: true)
  show_frequencies?: boolean;         // Include pattern frequencies (default: true)
  show_propagation?: boolean;         // Show error propagation chains (default: false)
  max_results?: number;               // Limit results (default: 50)
}
```

**Response**:

```typescript
interface ErrorPatternsResponse {
  scope: {
    type: 'file' | 'symbol' | 'project';
    path: string;
  };
  summary: ErrorSummary;
  error_types: ErrorTypeInfo[];       // Cataloged error types
  panic_sites: PanicSite[];           // Dangerous unwrap/panic calls
  error_patterns: ErrorPattern[];     // Common handling patterns
  propagation_chains?: PropagationChain[];
}

interface ErrorSummary {
  total_error_sites: number;
  by_type: {
    result_returns: number;
    panics: number;
    unwraps: number;
    expects: number;
  };
  by_severity: {
    low: number;
    medium: number;
    high: number;
    critical: number;
  };
  unique_error_types: number;
}

interface ErrorTypeInfo {
  error_enum: string;                 // "ApiError", "io::Error"
  variants: ErrorVariant[];
  total_occurrences: number;
  most_common_handling: string;       // 'retry', 'propagate', etc.
}

interface ErrorVariant {
  name: string;                       // "NotFound", "NetworkError"
  occurrences: number;
  test_failures: number;              // How often seen in tests
  handling_patterns: string[];        // ['retry', 'fallback']
  examples: CodeExample[];
}

interface PanicSite {
  file: string;
  line: number;
  function: string;
  panic_type: 'panic!' | 'unwrap' | 'expect' | 'unreachable!';
  message?: string;
  code: string;
  risk_level: 'low' | 'medium' | 'high';
  reason: string;                     // Why this risk level
  suggestion?: string;                // How to fix
}

interface ErrorPattern {
  error_type: string;
  variant?: string;
  handling_strategy: 'retry' | 'fallback' | 'propagate' | 'log' | 'ignore';
  frequency: number;                  // Times this pattern appears
  description: string;
  examples: CodeExample[];
}

interface CodeExample {
  file: string;
  line: number;
  snippet: string;
  context?: string[];                 // Surrounding lines
}

interface PropagationChain {
  error_type: string;
  chain: PropagationStep[];
  transformations: number;            // How many times error is transformed
}

interface PropagationStep {
  function: string;
  file: string;
  line: number;
  method: '?' | 'match' | 'map_err' | 'context';
  transforms: boolean;                // Does it change error type?
}
```

**Example Usage**:

```typescript
// Find all error patterns in a file
const patterns = await find_error_patterns({
  file: "src/api/users.rs",
  show_examples: true,
  show_frequencies: true
});

// Find high-risk panic sites project-wide
const panics = await find_error_patterns({
  project: true,
  error_type: ["panic", "unwrap", "expect"],
  min_severity: "high",
  include_tests: false
});

// Analyze specific function's error handling
const funcErrors = await find_error_patterns({
  symbol: "authenticate_user",
  file: "src/auth.rs",
  show_propagation: true
});
```

---

## Implementation

### Component 1: Error Detector

```rust
// src/errors/detector.rs

use anyhow::Result;
use tree_sitter::{Parser, Query, QueryCursor, Node};
use crate::storage::Store;

pub struct ErrorDetector {
    rust_query: Query,
}

impl ErrorDetector {
    pub fn new() -> Result<Self> {
        let rust_source = r#"
            ; Result return types
            (function_item
              return_type: (type_identifier) @return_type
              (#match? @return_type "^Result")) @result_fn
            
            ; panic! macros
            (macro_invocation
              macro: (identifier) @macro_name
              (#eq? @macro_name "panic")
              (token_tree) @panic_msg) @panic_site
            
            ; unwrap() calls
            (call_expression
              function: (field_expression
                field: (field_identifier) @method_name
                (#eq? @method_name "unwrap"))) @unwrap_site
            
            ; expect() calls
            (call_expression
              function: (field_expression
                field: (field_identifier) @method_name
                (#eq? @method_name "expect"))
              arguments: (arguments
                (string_literal) @expect_msg)) @expect_site
            
            ; unreachable! macro
            (macro_invocation
              macro: (identifier) @macro_name
              (#eq? @macro_name "unreachable")) @unreachable_site
            
            ; ? operator
            (try_expression) @try_op
            
            ; Match on Result
            (match_expression
              value: (_) @match_val
              (#match? @match_val ".*")
              body: (match_block) @match_body) @result_match
        "#;

        Ok(Self {
            rust_query: Query::new(&tree_sitter_rust::language(), rust_source)?,
        })
    }

    /// Detect all error sites in a file
    pub async fn detect_errors(&self, file_id: i64, content: &[u8]) -> Result<Vec<ErrorSite>> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::language())?;
        
        let tree = parser.parse(content, None)
            .ok_or_else(|| anyhow::anyhow!("Parse failed"))?;
        
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&self.rust_query, tree.root_node(), content);
        
        let mut sites = Vec::new();
        for m in matches {
            if let Some(site) = self.extract_error_site(m, content, file_id) {
                sites.push(site);
            }
        }
        
        Ok(sites)
    }

    fn extract_error_site(
        &self,
        query_match: tree_sitter::QueryMatch,
        content: &[u8],
        file_id: i64,
    ) -> Option<ErrorSite> {
        let capture = query_match.captures.first()?;
        let node = capture.node;
        let line = node.start_position().row + 1;
        
        // Determine error type based on capture name
        let capture_name = self.rust_query.capture_names()[capture.index as usize];
        
        let (error_type, severity) = match capture_name {
            "panic_site" => {
                let msg = self.extract_panic_message(node, content);
                return Some(ErrorSite {
                    file_id,
                    line,
                    error_type: ErrorType::Panic,
                    code_snippet: node.utf8_text(content).ok()?.to_string(),
                    message: Some(msg),
                    severity: Severity::High,
                    ..Default::default()
                });
            }
            "unwrap_site" => {
                let risk = self.assess_unwrap_risk(node, content);
                return Some(ErrorSite {
                    file_id,
                    line,
                    error_type: ErrorType::Unwrap,
                    code_snippet: node.utf8_text(content).ok()?.to_string(),
                    severity: risk,
                    ..Default::default()
                });
            }
            "expect_site" => {
                let msg = self.extract_expect_message(node, content);
                return Some(ErrorSite {
                    file_id,
                    line,
                    error_type: ErrorType::Expect,
                    code_snippet: node.utf8_text(content).ok()?.to_string(),
                    message: Some(msg),
                    severity: Severity::Medium,
                    ..Default::default()
                });
            }
            "result_fn" => {
                let (error_enum, variant) = self.extract_error_type(node, content)?;
                return Some(ErrorSite {
                    file_id,
                    line,
                    error_type: ErrorType::Result,
                    error_enum: Some(error_enum),
                    error_variant: variant,
                    code_snippet: node.utf8_text(content).ok()?.to_string(),
                    severity: Severity::Low,
                    ..Default::default()
                });
            }
            _ => return None,
        };
        
        None
    }

    fn extract_panic_message(&self, node: Node, content: &[u8]) -> String {
        // Extract message from panic!("message")
        node.child_by_field_name("arguments")
            .and_then(|args| args.named_child(0))
            .and_then(|msg| msg.utf8_text(content).ok())
            .unwrap_or("(no message)")
            .to_string()
    }

    fn extract_expect_message(&self, node: Node, content: &[u8]) -> String {
        node.child_by_field_name("arguments")
            .and_then(|args| args.named_child(0))
            .and_then(|msg| msg.utf8_text(content).ok())
            .unwrap_or("(no message)")
            .to_string()
    }

    fn extract_error_type(&self, node: Node, content: &[u8]) -> Option<(String, Option<String>)> {
        // Parse Result<T, E> to extract E
        let return_type_node = node.child_by_field_name("return_type")?;
        let type_text = return_type_node.utf8_text(content).ok()?;
        
        // Simple regex parsing: Result<T, ErrorType>
        if let Some(start) = type_text.rfind(',') {
            let error_part = &type_text[start + 1..].trim_end_matches('>').trim();
            
            // Check if it's an enum variant like "ApiError::NotFound"
            if let Some(variant_pos) = error_part.find("::") {
                let enum_name = error_part[..variant_pos].to_string();
                let variant = error_part[variant_pos + 2..].to_string();
                return Some((enum_name, Some(variant)));
            }
            
            return Some((error_part.to_string(), None));
        }
        
        None
    }

    fn assess_unwrap_risk(&self, node: Node, content: &[u8]) -> Severity {
        // Check if unwrap is guarded by if/match
        if self.is_guarded(node) {
            return Severity::Low;
        }
        
        // Check if in test code
        if self.is_in_test(node, content) {
            return Severity::Low;
        }
        
        // Unguarded unwrap in prod code = high risk
        Severity::High
    }

    fn is_guarded(&self, node: Node) -> bool {
        // Walk up to see if inside if let Some or match
        let mut current = node;
        while let Some(parent) = current.parent() {
            match parent.kind() {
                "if_let_expression" | "match_arm" => return true,
                _ => {}
            }
            current = parent;
        }
        false
    }

    fn is_in_test(&self, node: Node, content: &[u8]) -> bool {
        // Check if inside #[test] or #[cfg(test)]
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "attribute_item" {
                if let Ok(attr_text) = parent.utf8_text(content) {
                    if attr_text.contains("#[test]") || attr_text.contains("#[cfg(test)]") {
                        return true;
                    }
                }
            }
            current = parent;
        }
        false
    }
}

#[derive(Debug, Clone)]
pub struct ErrorSite {
    pub file_id: i64,
    pub symbol_id: Option<i64>,
    pub line: usize,
    pub error_type: ErrorType,
    pub error_enum: Option<String>,
    pub error_variant: Option<String>,
    pub code_snippet: String,
    pub message: Option<String>,
    pub severity: Severity,
    pub is_test: bool,
}

impl Default for ErrorSite {
    fn default() -> Self {
        Self {
            file_id: 0,
            symbol_id: None,
            line: 0,
            error_type: ErrorType::Result,
            error_enum: None,
            error_variant: None,
            code_snippet: String::new(),
            message: None,
            severity: Severity::Low,
            is_test: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorType {
    Result,
    Panic,
    Unwrap,
    Expect,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}
```

### Component 2: Pattern Analyzer

```rust
// src/errors/pattern_analyzer.rs

use std::collections::HashMap;
use anyhow::Result;
use crate::storage::Store;
use super::detector::ErrorSite;

pub struct PatternAnalyzer {
    store: Store,
}

impl PatternAnalyzer {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Analyze error handling patterns across error sites
    pub async fn analyze_patterns(&self, sites: &[ErrorSite]) -> Result<Vec<ErrorPattern>> {
        // Group by error type
        let mut grouped: HashMap<String, Vec<&ErrorSite>> = HashMap::new();
        
        for site in sites {
            if let Some(ref error_enum) = site.error_enum {
                let key = format!(
                    "{}::{}",
                    error_enum,
                    site.error_variant.as_deref().unwrap_or("*")
                );
                grouped.entry(key).or_default().push(site);
            }
        }
        
        // Analyze each group
        let mut patterns = Vec::new();
        for (error_key, group) in grouped {
            let pattern = self.extract_pattern(&error_key, group).await?;
            patterns.push(pattern);
        }
        
        // Sort by frequency
        patterns.sort_by(|a, b| b.frequency.cmp(&a.frequency));
        
        Ok(patterns)
    }

    async fn extract_pattern(
        &self,
        error_key: &str,
        sites: Vec<&ErrorSite>,
    ) -> Result<ErrorPattern> {
        // Count handling strategies
        let mut strategy_counts: HashMap<HandlingStrategy, usize> = HashMap::new();
        let mut examples = Vec::new();
        
        for site in &sites {
            let strategy = self.identify_handling_strategy(site).await?;
            *strategy_counts.entry(strategy).or_default() += 1;
            
            // Keep up to 3 examples
            if examples.len() < 3 {
                examples.push(self.build_code_example(site).await?);
            }
        }
        
        // Most common strategy
        let most_common = strategy_counts.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(strategy, _)| *strategy)
            .unwrap_or(HandlingStrategy::Propagate);
        
        let parts: Vec<&str> = error_key.split("::").collect();
        
        Ok(ErrorPattern {
            error_enum: parts[0].to_string(),
            error_variant: if parts.len() > 1 && parts[1] != "*" {
                Some(parts[1].to_string())
            } else {
                None
            },
            handling_strategy: most_common,
            frequency: sites.len(),
            description: self.generate_description(&most_common, sites.len()),
            examples,
        })
    }

    async fn identify_handling_strategy(&self, site: &ErrorSite) -> Result<HandlingStrategy> {
        // Read code around error site to determine handling
        let conn = self.store.conn().await?;
        let file_path: String = conn.query_row(
            "SELECT path FROM files WHERE id = ?",
            [site.file_id],
            |row| row.get(0),
        )?;
        
        let content = tokio::fs::read_to_string(&file_path).await?;
        let lines: Vec<&str> = content.lines().collect();
        
        if site.line == 0 || site.line > lines.len() {
            return Ok(HandlingStrategy::Unknown);
        }
        
        // Look at surrounding code
        let start = site.line.saturating_sub(5);
        let end = (site.line + 5).min(lines.len());
        let context = lines[start..end].join("\n");
        
        // Pattern matching
        if context.contains("retry") || context.contains("loop") {
            Ok(HandlingStrategy::Retry)
        } else if context.contains("unwrap_or") || context.contains("unwrap_or_else") {
            Ok(HandlingStrategy::Fallback)
        } else if context.contains("?") {
            Ok(HandlingStrategy::Propagate)
        } else if context.contains("log::") || context.contains("eprintln!") {
            Ok(HandlingStrategy::Log)
        } else if context.contains("let _ =") {
            Ok(HandlingStrategy::Ignore)
        } else {
            Ok(HandlingStrategy::Unknown)
        }
    }

    async fn build_code_example(&self, site: &ErrorSite) -> Result<CodeExample> {
        let conn = self.store.conn().await?;
        let file_path: String = conn.query_row(
            "SELECT path FROM files WHERE id = ?",
            [site.file_id],
            |row| row.get(0),
        )?;
        
        Ok(CodeExample {
            file: file_path,
            line: site.line,
            snippet: site.code_snippet.clone(),
            context: None,
        })
    }

    fn generate_description(&self, strategy: &HandlingStrategy, freq: usize) -> String {
        match strategy {
            HandlingStrategy::Retry => {
                format!("Errors are typically retried (seen {} times)", freq)
            }
            HandlingStrategy::Fallback => {
                format!("Errors use fallback values (seen {} times)", freq)
            }
            HandlingStrategy::Propagate => {
                format!("Errors are propagated with ? operator (seen {} times)", freq)
            }
            HandlingStrategy::Log => {
                format!("Errors are logged and handled (seen {} times)", freq)
            }
            HandlingStrategy::Ignore => {
                format!("Errors are ignored with let _ = (seen {} times)", freq)
            }
            HandlingStrategy::Unknown => {
                format!("Handling strategy unclear (seen {} times)", freq)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorPattern {
    pub error_enum: String,
    pub error_variant: Option<String>,
    pub handling_strategy: HandlingStrategy,
    pub frequency: usize,
    pub description: String,
    pub examples: Vec<CodeExample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlingStrategy {
    Retry,
    Fallback,
    Propagate,
    Log,
    Ignore,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct CodeExample {
    pub file: String,
    pub line: usize,
    pub snippet: String,
    pub context: Option<Vec<String>>,
}
```

### Component 3: MCP Tool Handler

```rust
// src/tools/error_patterns.rs

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::errors::{ErrorDetector, PatternAnalyzer};
use crate::storage::Store;

#[derive(Debug, Deserialize)]
pub struct FindErrorPatternsParams {
    pub file: Option<String>,
    pub symbol: Option<String>,
    pub project: Option<bool>,
    pub error_type: Option<Vec<String>>,
    pub min_severity: Option<String>,
    pub include_tests: Option<bool>,
    pub show_examples: Option<bool>,
    pub show_frequencies: Option<bool>,
    pub show_propagation: Option<bool>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ErrorPatternsResponse {
    pub scope: Scope,
    pub summary: ErrorSummary,
    pub error_types: Vec<ErrorTypeInfo>,
    pub panic_sites: Vec<PanicSite>,
    pub error_patterns: Vec<ErrorPatternResponse>,
}

pub async fn find_error_patterns(
    params: FindErrorPatternsParams,
    store: Store,
) -> Result<ErrorPatternsResponse> {
    let detector = ErrorDetector::new()?;
    let analyzer = PatternAnalyzer::new(store.clone());
    
    // Determine scope
    let (scope, file_ids) = if let Some(file) = params.file {
        let file_id = store.get_file_id(&file).await?
            .ok_or_else(|| anyhow::anyhow!("File not found"))?;
        (Scope { scope_type: "file".into(), path: file }, vec![file_id])
    } else if params.project.unwrap_or(false) {
        (Scope { scope_type: "project".into(), path: ".".into() }, store.get_all_file_ids().await?)
    } else {
        return Err(anyhow::anyhow!("Must specify file or project"));
    };
    
    // Detect errors in all files
    let mut all_sites = Vec::new();
    for file_id in file_ids {
        let file_path = store.get_file_path(file_id).await?;
        let content = tokio::fs::read(&file_path).await?;
        let sites = detector.detect_errors(file_id, &content).await?;
        all_sites.extend(sites);
    }
    
    // Filter by error type
    if let Some(types) = &params.error_type {
        all_sites.retain(|site| {
            let type_str = format!("{:?}", site.error_type).to_lowercase();
            types.iter().any(|t| t.to_lowercase() == type_str)
        });
    }
    
    // Filter by severity
    if let Some(min_sev) = &params.min_severity {
        let min = parse_severity(min_sev);
        all_sites.retain(|site| site.severity >= min);
    }
    
    // Analyze patterns
    let patterns = analyzer.analyze_patterns(&all_sites).await?;
    
    // Build summary
    let summary = build_summary(&all_sites);
    
    // Extract panic sites
    let panic_sites = extract_panic_sites(&all_sites, &store).await?;
    
    // Build response
    Ok(ErrorPatternsResponse {
        scope,
        summary,
        error_types: vec![], // TODO: implement
        panic_sites,
        error_patterns: patterns.into_iter().map(|p| p.into()).collect(),
    })
}

fn parse_severity(s: &str) -> Severity {
    match s.to_lowercase().as_str() {
        "low" => Severity::Low,
        "medium" => Severity::Medium,
        "high" => Severity::High,
        "critical" => Severity::Critical,
        _ => Severity::Low,
    }
}

fn build_summary(sites: &[ErrorSite]) -> ErrorSummary {
    let mut by_type = TypeBreakdown::default();
    let mut by_severity = SeverityBreakdown::default();
    
    for site in sites {
        match site.error_type {
            ErrorType::Result => by_type.result_returns += 1,
            ErrorType::Panic => by_type.panics += 1,
            ErrorType::Unwrap => by_type.unwraps += 1,
            ErrorType::Expect => by_type.expects += 1,
            _ => {}
        }
        
        match site.severity {
            Severity::Low => by_severity.low += 1,
            Severity::Medium => by_severity.medium += 1,
            Severity::High => by_severity.high += 1,
            Severity::Critical => by_severity.critical += 1,
        }
    }
    
    ErrorSummary {
        total_error_sites: sites.len(),
        by_type,
        by_severity,
        unique_error_types: 0, // TODO
    }
}

async fn extract_panic_sites(sites: &[ErrorSite], store: &Store) -> Result<Vec<PanicSite>> {
    let mut panic_sites = Vec::new();
    
    for site in sites {
        if matches!(site.error_type, ErrorType::Panic | ErrorType::Unwrap | ErrorType::Expect) {
            let file_path = store.get_file_path(site.file_id).await?;
            
            panic_sites.push(PanicSite {
                file: file_path,
                line: site.line,
                function: "unknown".into(), // TODO: lookup
                panic_type: format!("{:?}", site.error_type).to_lowercase(),
                message: site.message.clone(),
                code: site.code_snippet.clone(),
                risk_level: format!("{:?}", site.severity).to_lowercase(),
                reason: generate_risk_reason(&site),
                suggestion: Some(generate_suggestion(&site)),
            });
        }
    }
    
    Ok(panic_sites)
}

fn generate_risk_reason(site: &ErrorSite) -> String {
    match site.error_type {
        ErrorType::Unwrap if site.severity == Severity::High => {
            "Unguarded unwrap() in production code can panic".into()
        }
        ErrorType::Panic => "Explicit panic will crash the program".into(),
        _ => "Potential panic site".into(),
    }
}

fn generate_suggestion(site: &ErrorSite) -> String {
    match site.error_type {
        ErrorType::Unwrap => "Replace with match, if let, or unwrap_or()".into(),
        ErrorType::Expect => "Use proper error handling instead of expect()".into(),
        ErrorType::Panic => "Return Result<T, E> instead of panicking".into(),
        _ => "Handle error gracefully".into(),
    }
}

// Response types
#[derive(Debug, Serialize)]
struct Scope {
    scope_type: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct ErrorSummary {
    total_error_sites: usize,
    by_type: TypeBreakdown,
    by_severity: SeverityBreakdown,
    unique_error_types: usize,
}

#[derive(Debug, Serialize, Default)]
struct TypeBreakdown {
    result_returns: usize,
    panics: usize,
    unwraps: usize,
    expects: usize,
}

#[derive(Debug, Serialize, Default)]
struct SeverityBreakdown {
    low: usize,
    medium: usize,
    high: usize,
    critical: usize,
}

#[derive(Debug, Serialize)]
struct PanicSite {
    file: String,
    line: usize,
    function: String,
    panic_type: String,
    message: Option<String>,
    code: String,
    risk_level: String,
    reason: String,
    suggestion: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorPatternResponse {
    error_type: String,
    variant: Option<String>,
    handling_strategy: String,
    frequency: usize,
    description: String,
    examples: Vec<CodeExample>,
}

impl From<ErrorPattern> for ErrorPatternResponse {
    fn from(p: ErrorPattern) -> Self {
        Self {
            error_type: p.error_enum,
            variant: p.error_variant,
            handling_strategy: format!("{:?}", p.handling_strategy).to_lowercase(),
            frequency: p.frequency,
            description: p.description,
            examples: p.examples,
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorTypeInfo {
    error_enum: String,
    total_occurrences: usize,
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
    fn test_detect_unwrap() {
        let detector = ErrorDetector::new().unwrap();
        let code = b"let x = some_result.unwrap();";
        
        // Parse and detect
        let sites = detector.detect_errors(1, code).await.unwrap();
        
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].error_type, ErrorType::Unwrap);
    }

    #[test]
    fn test_risk_assessment() {
        let detector = ErrorDetector::new().unwrap();
        
        // Guarded unwrap = low risk
        let guarded = b"if let Some(x) = opt { x.unwrap() }";
        // Unguarded unwrap = high risk
        let unguarded = b"let x = opt.unwrap();";
        
        // Test risk scoring...
    }
}
```

---

## Success Metrics

- **Detection accuracy**: > 98% of error sites found
- **Pattern accuracy**: > 90% correct handling strategy classification
- **Panic identification**: 100% of unwrap/expect/panic found
- **Performance**: < 5s for project-wide analysis (500 files)

---

## Usage Examples

```bash
# Find all panic risks in production code
mcp query find_error_patterns '{
  "project": true,
  "error_type": ["panic", "unwrap", "expect"],
  "min_severity": "high",
  "include_tests": false
}'

# Analyze error handling in specific file
mcp query find_error_patterns '{
  "file": "src/api/users.rs",
  "show_examples": true,
  "show_frequencies": true
}'
```

**Response**:
```json
{
  "scope": {"type": "file", "path": "src/api/users.rs"},
  "summary": {
    "total_error_sites": 12,
    "by_type": {"result_returns": 8, "unwraps": 3, "expects": 1},
    "by_severity": {"low": 5, "medium": 4, "high": 3}
  },
  "panic_sites": [
    {
      "file": "src/api/users.rs",
      "line": 156,
      "function": "get_user_profile",
      "panic_type": "unwrap",
      "code": "user.profile.unwrap()",
      "risk_level": "high",
      "reason": "Unguarded unwrap() in production code",
      "suggestion": "Replace with match or unwrap_or_default()"
    }
  ],
  "error_patterns": [
    {
      "error_type": "ApiError",
      "variant": "NotFound",
      "handling_strategy": "fallback",
      "frequency": 5,
      "description": "Errors use fallback values (seen 5 times)",
      "examples": [...]
    }
  ]
}
```
