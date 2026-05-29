# Feature: Get Test Coverage

**Feature ID**: PHASE1-009  
**Priority**: 🔥🔥🔥 HIGH  
**Estimated Effort**: 5 days  
**Phase**: 1 - Runtime & Evolution Context  
**Dependencies**: Index infrastructure (PHASE0-001, PHASE0-002)

---

## Overview

Test coverage tracking provides visibility into what code is tested, what isn't, and the quality of test coverage across the codebase. This feature analyzes test files, identifies tested functions/modules, and calculates coverage metrics without requiring code execution.

### Problem Statement

Developers currently face several challenges:

1. **No visibility**: Can't easily see which functions/modules have tests
2. **Regression risk**: Refactoring code without knowing test coverage is risky
3. **Test gaps**: Hard to identify untested critical paths
4. **Coverage drift**: No way to track coverage changes over time
5. **Manual analysis**: Must manually search for test files and correlate to source

Example scenario:
```rust
// src/api/users.rs
pub fn create_user(name: &str) -> Result<User> { ... }
pub fn delete_user(id: i64) -> Result<()> { ... }
pub fn update_user(id: i64, name: &str) -> Result<User> { ... }

// tests/api_tests.rs
#[test]
fn test_create_user() { ... }
// ❌ No test for delete_user!
// ❌ No test for update_user!
```

Developer wants to know: "Which functions in `users.rs` have tests?"

### Goals

1. ✅ **Static coverage analysis**: Identify tested symbols without running tests
2. ✅ **Multiple granularities**: File-level, module-level, function-level coverage
3. ✅ **Test discovery**: Find all test files and test functions
4. ✅ **Symbol correlation**: Map tests to source functions they exercise
5. ✅ **Coverage metrics**: Calculate percentages, identify gaps
6. ✅ **Cross-references**: Show which tests cover which code
7. ✅ **Performance**: < 2s for file-level, < 5s for project-wide

### Non-Goals

1. ❌ **Runtime coverage**: Not instrumenting code or measuring execution coverage (like `cargo tarpaulin`)
2. ❌ **Line coverage**: Not tracking line-by-line coverage (requires execution)
3. ❌ **Branch coverage**: Not analyzing conditional branches
4. ❌ **Test execution**: Not running tests, only analyzing structure
5. ❌ **Code quality**: Not evaluating test quality or assertions

---

## Architecture

### System Components

```
┌─────────────────────────────────────────────────────────────┐
│                     Test Coverage System                     │
└─────────────────────────────────────────────────────────────┘
                              │
                ┌─────────────┴─────────────┐
                │                           │
        ┌───────▼────────┐         ┌────────▼────────┐
        │  Test Analyzer │         │ Coverage Engine │
        └───────┬────────┘         └────────┬────────┘
                │                           │
    ┌───────────┼───────────┐               │
    │           │           │               │
┌───▼───┐  ┌───▼───┐  ┌────▼────┐   ┌──────▼──────┐
│ Test  │  │ Call  │  │ Import  │   │ Coverage    │
│Finder │  │Tracer │  │ Analyzer│   │ Calculator  │
└───┬───┘  └───┬───┘  └────┬────┘   └──────┬──────┘
    │          │           │               │
    └──────────┴───────────┴───────────────┘
                    │
            ┌───────▼────────┐
            │  SQLite Index  │
            │  - symbols     │
            │  - references  │
            │  - files       │
            └────────────────┘
```

### Data Model

```sql
-- New table for test metadata
CREATE TABLE test_functions (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    symbol_id INTEGER NOT NULL,  -- References symbols.id
    test_name TEXT NOT NULL,
    test_type TEXT NOT NULL,     -- 'unit', 'integration', 'e2e', 'benchmark'
    is_async BOOLEAN DEFAULT FALSE,
    attributes TEXT,              -- JSON: ["#[tokio::test]", "#[should_panic]"]
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
);

-- Test-to-source correlation
CREATE TABLE test_coverage_map (
    id INTEGER PRIMARY KEY,
    test_id INTEGER NOT NULL,       -- test_functions.id
    covered_symbol_id INTEGER NOT NULL,  -- symbols.id of tested function
    coverage_type TEXT NOT NULL,    -- 'direct_call', 'indirect_call', 'import_only'
    confidence REAL NOT NULL,       -- 0.0-1.0
    call_chain TEXT,                -- JSON: ["test_fn", "helper", "target"]
    FOREIGN KEY (test_id) REFERENCES test_functions(id) ON DELETE CASCADE,
    FOREIGN KEY (covered_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
);

CREATE INDEX idx_test_functions_file ON test_functions(file_id);
CREATE INDEX idx_test_coverage_covered ON test_coverage_map(covered_symbol_id);
CREATE INDEX idx_test_coverage_test ON test_coverage_map(test_id);

-- Coverage cache for performance
CREATE TABLE coverage_cache (
    id INTEGER PRIMARY KEY,
    target_type TEXT NOT NULL,     -- 'file', 'module', 'project'
    target_path TEXT NOT NULL,
    coverage_data TEXT NOT NULL,   -- JSON
    computed_at INTEGER NOT NULL,  -- Unix timestamp
    version INTEGER NOT NULL       -- Incremented on reindex
);
CREATE INDEX idx_coverage_cache_lookup ON coverage_cache(target_type, target_path);
```

### Coverage Calculation Strategy

**Three-stage analysis**:

1. **Test Discovery** (TestFinder)
   - Find all test files: `*_test.rs`, `tests/*.rs`, `*_test.py`, `*.test.ts`, etc.
   - Parse test functions: `#[test]`, `#[tokio::test]`, `describe()`, `it()`, `def test_*()`
   - Extract test metadata: attributes, async markers, test type

2. **Symbol Correlation** (CallTracer + ImportAnalyzer)
   - **Direct calls**: Test function calls source function directly
   - **Indirect calls**: Test calls helper that calls source (up to 3 levels deep)
   - **Import analysis**: Test imports module containing source
   - Confidence scoring:
     - Direct call: 1.0
     - Indirect call (1 hop): 0.8
     - Indirect call (2 hops): 0.6
     - Indirect call (3+ hops): 0.4
     - Import only: 0.2

3. **Coverage Metrics** (CoverageCalculator)
   - Aggregation levels: function → module → file → project
   - Metrics:
     - `tested_count`: Number of symbols with tests (confidence ≥ 0.5)
     - `total_count`: Total public symbols
     - `coverage_percentage`: (tested / total) × 100
     - `untested_symbols`: List of symbols without tests

---

## API Specification

### Tool: `get_test_coverage`

**Description**: Analyzes test coverage for a file, module, or entire project.

**Parameters**:

```typescript
interface GetTestCoverageParams {
  // What to analyze (mutually exclusive)
  file?: string;           // Path to source file
  module?: string;         // Module path (e.g., "crate::api::users")
  project?: boolean;       // Analyze entire project
  
  // Options
  include_private?: boolean;   // Include private symbols (default: false)
  min_confidence?: number;     // Minimum confidence 0.0-1.0 (default: 0.5)
  show_test_details?: boolean; // Include test function details (default: false)
  show_untested?: boolean;     // Include list of untested symbols (default: true)
}
```

**Response**:

```typescript
interface TestCoverageResponse {
  target: {
    type: 'file' | 'module' | 'project';
    path: string;
  };
  summary: CoverageSummary;
  by_file?: FileCoverage[];           // If project-level
  by_module?: ModuleCoverage[];       // If project-level
  untested_symbols?: UntestedSymbol[];
  test_files: TestFileInfo[];
  computed_at: string;                // ISO timestamp
}

interface CoverageSummary {
  total_symbols: number;
  tested_symbols: number;
  untested_symbols: number;
  coverage_percentage: number;        // 0-100
  test_count: number;                 // Total test functions
  by_type: {
    unit_tests: number;
    integration_tests: number;
    e2e_tests: number;
    benchmarks: number;
  };
}

interface FileCoverage {
  file: string;
  summary: CoverageSummary;
  symbols: SymbolCoverage[];
}

interface SymbolCoverage {
  name: string;
  kind: string;                       // 'function', 'struct', 'impl', etc.
  is_public: boolean;
  is_tested: boolean;
  test_count: number;
  tests: TestReference[];             // If show_test_details=true
  coverage_confidence: number;        // 0.0-1.0
}

interface TestReference {
  test_name: string;
  test_file: string;
  test_line: number;
  coverage_type: 'direct_call' | 'indirect_call' | 'import_only';
  confidence: number;
  call_chain?: string[];              // For indirect calls
}

interface UntestedSymbol {
  name: string;
  kind: string;
  file: string;
  line: number;
  is_public: boolean;
  suggestion?: string;                // Suggested test name
}

interface TestFileInfo {
  path: string;
  test_count: number;
  coverage_contribution: number;      // How many symbols this test file covers
}
```

**Example Usage**:

```typescript
// File-level coverage
const coverage = await get_test_coverage({
  file: "src/api/users.rs",
  show_test_details: true,
  show_untested: true
});

// Module-level coverage
const modCoverage = await get_test_coverage({
  module: "crate::api",
  include_private: false
});

// Project-wide coverage
const projectCoverage = await get_test_coverage({
  project: true,
  min_confidence: 0.6
});
```

---

## Implementation

### Component 1: Test Finder

```rust
// src/coverage/test_finder.rs

use anyhow::Result;
use tree_sitter::{Parser, Query, QueryCursor};
use crate::storage::models::{File, Symbol};

pub struct TestFinder {
    rust_query: Query,
    typescript_query: Query,
    python_query: Query,
}

impl TestFinder {
    pub fn new() -> Result<Self> {
        let rust_source = r#"
            ; Rust test functions
            (attribute_item
              (attribute
                (identifier) @attr_name
                (#match? @attr_name "^(test|tokio::test|async_test)$"))
              (function_item
                name: (identifier) @test_name)) @test_fn
            
            ; Benchmark functions
            (attribute_item
              (attribute
                (identifier) @attr_name
                (#eq? @attr_name "bench"))
              (function_item
                name: (identifier) @bench_name)) @bench_fn
        "#;

        let typescript_source = r#"
            ; Jest/Mocha tests
            (call_expression
              function: (identifier) @test_fn_name
              (#match? @test_fn_name "^(test|it|describe)$")
              arguments: (arguments
                (string) @test_description
                (arrow_function) @test_body))
        "#;

        let python_source = r#"
            ; Pytest functions
            (function_definition
              name: (identifier) @test_name
              (#match? @test_name "^test_"))
            
            ; Unittest methods
            (class_definition
              body: (block
                (function_definition
                  name: (identifier) @test_method
                  (#match? @test_method "^test_"))))
        "#;

        Ok(Self {
            rust_query: Query::new(&tree_sitter_rust::language(), rust_source)?,
            typescript_query: Query::new(&tree_sitter_typescript::language_typescript(), typescript_source)?,
            python_query: Query::new(&tree_sitter_python::language(), python_source)?,
        })
    }

    /// Find all test functions in a file
    pub fn find_tests(&self, file: &File, content: &[u8]) -> Result<Vec<TestFunction>> {
        let lang = Language::from_extension(&file.path);
        let mut parser = Parser::new();
        parser.set_language(&lang.tree_sitter_lang())?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse file"))?;

        let query = match lang {
            Language::Rust => &self.rust_query,
            Language::TypeScript | Language::JavaScript => &self.typescript_query,
            Language::Python => &self.python_query,
            _ => return Ok(Vec::new()), // No test support for this language
        };

        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(query, tree.root_node(), content);

        let mut tests = Vec::new();
        for m in matches {
            let test = self.extract_test_metadata(m, content, &lang)?;
            tests.push(test);
        }

        Ok(tests)
    }

    fn extract_test_metadata(
        &self,
        query_match: tree_sitter::QueryMatch,
        content: &[u8],
        lang: &Language,
    ) -> Result<TestFunction> {
        let test_name = query_match.captures.iter()
            .find(|c| c.node.kind().contains("name"))
            .map(|c| c.node.utf8_text(content).unwrap_or(""))
            .unwrap_or("");

        let attributes = self.extract_attributes(&query_match, content);
        let test_type = self.infer_test_type(&attributes, lang);
        let is_async = attributes.iter().any(|a| 
            a.contains("tokio::test") || a.contains("async_test") || a.contains("async")
        );

        Ok(TestFunction {
            name: test_name.to_string(),
            test_type,
            is_async,
            attributes,
            start_line: query_match.captures[0].node.start_position().row + 1,
            end_line: query_match.captures[0].node.end_position().row + 1,
        })
    }

    fn extract_attributes(&self, query_match: &tree_sitter::QueryMatch, content: &[u8]) -> Vec<String> {
        query_match.captures.iter()
            .filter(|c| c.node.kind() == "attribute_item")
            .map(|c| c.node.utf8_text(content).unwrap_or("").to_string())
            .collect()
    }

    fn infer_test_type(&self, attributes: &[String], lang: &Language) -> TestType {
        // Rust
        if attributes.iter().any(|a| a.contains("#[bench]")) {
            return TestType::Benchmark;
        }
        if attributes.iter().any(|a| a.contains("#[test]")) {
            return TestType::Unit;
        }

        // TypeScript
        if lang.is_typescript() {
            if attributes.iter().any(|a| a.contains("describe")) {
                return TestType::Integration;
            }
            return TestType::Unit;
        }

        // Python
        if lang.is_python() {
            return TestType::Unit; // Could be more sophisticated
        }

        TestType::Unit
    }
}

#[derive(Debug, Clone)]
pub struct TestFunction {
    pub name: String,
    pub test_type: TestType,
    pub is_async: bool,
    pub attributes: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestType {
    Unit,
    Integration,
    E2E,
    Benchmark,
}
```

### Component 2: Call Tracer

```rust
// src/coverage/call_tracer.rs

use std::collections::{HashMap, HashSet, VecDeque};
use anyhow::Result;
use crate::storage::Store;

pub struct CallTracer {
    store: Store,
    max_depth: usize, // Maximum call chain depth (default: 3)
}

impl CallTracer {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            max_depth: 3,
        }
    }

    /// Trace all symbols called by a test function
    /// Returns: Map of (target_symbol_id -> (confidence, call_chain))
    pub async fn trace_coverage(
        &self,
        test_symbol_id: i64,
    ) -> Result<HashMap<i64, CoverageTrace>> {
        let mut coverage = HashMap::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        // Start BFS from test function
        queue.push_back((test_symbol_id, vec![test_symbol_id], 0));

        while let Some((current_id, path, depth)) = queue.pop_front() {
            if visited.contains(&current_id) || depth > self.max_depth {
                continue;
            }
            visited.insert(current_id);

            // Get all callees of current symbol
            let callees = self.store.get_callees(current_id).await?;

            for callee in callees {
                let new_path = {
                    let mut p = path.clone();
                    p.push(callee.id);
                    p
                };

                let confidence = self.calculate_confidence(depth + 1);
                let coverage_type = if depth == 0 {
                    CoverageType::DirectCall
                } else {
                    CoverageType::IndirectCall
                };

                // Record coverage (if not test function itself)
                if callee.id != test_symbol_id && !self.is_test_function(callee.id).await? {
                    coverage.entry(callee.id)
                        .and_modify(|existing: &mut CoverageTrace| {
                            // Keep trace with higher confidence
                            if confidence > existing.confidence {
                                existing.confidence = confidence;
                                existing.call_chain = new_path.clone();
                                existing.coverage_type = coverage_type;
                            }
                        })
                        .or_insert(CoverageTrace {
                            confidence,
                            coverage_type,
                            call_chain: new_path.clone(),
                        });
                }

                // Continue BFS
                if depth + 1 < self.max_depth {
                    queue.push_back((callee.id, new_path, depth + 1));
                }
            }
        }

        Ok(coverage)
    }

    fn calculate_confidence(&self, depth: usize) -> f64 {
        match depth {
            0 => 1.0,     // Direct call
            1 => 0.8,     // 1 hop
            2 => 0.6,     // 2 hops
            _ => 0.4,     // 3+ hops
        }
    }

    async fn is_test_function(&self, symbol_id: i64) -> Result<bool> {
        let conn = self.store.conn().await?;
        let result: Option<i64> = conn.query_row(
            "SELECT 1 FROM test_functions WHERE symbol_id = ?",
            [symbol_id],
            |row| row.get(0),
        ).optional()?;
        Ok(result.is_some())
    }
}

#[derive(Debug, Clone)]
pub struct CoverageTrace {
    pub confidence: f64,
    pub coverage_type: CoverageType,
    pub call_chain: Vec<i64>, // Symbol IDs from test to target
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoverageType {
    DirectCall,
    IndirectCall,
    ImportOnly,
}
```

### Component 3: Coverage Calculator

```rust
// src/coverage/calculator.rs

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::storage::Store;

pub struct CoverageCalculator {
    store: Store,
    min_confidence: f64,
}

impl CoverageCalculator {
    pub fn new(store: Store, min_confidence: f64) -> Self {
        Self {
            store,
            min_confidence,
        }
    }

    /// Calculate coverage for a specific file
    pub async fn calculate_file_coverage(
        &self,
        file_path: &str,
        include_private: bool,
    ) -> Result<FileCoverage> {
        let conn = self.store.conn().await?;

        // Get all symbols in file
        let symbols: Vec<(i64, String, String, bool)> = conn.prepare(
            "SELECT s.id, s.name, s.kind, s.is_public
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE f.path = ?
             AND (? OR s.is_public = TRUE)"
        )?.query_map([file_path, include_private], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;

        let total_symbols = symbols.len();
        let mut tested_symbols = 0;
        let mut symbol_coverage = Vec::new();

        for (symbol_id, name, kind, is_public) in symbols {
            // Get test coverage for this symbol
            let tests = self.get_symbol_tests(symbol_id).await?;
            let is_tested = !tests.is_empty();
            
            if is_tested {
                tested_symbols += 1;
            }

            let max_confidence = tests.iter()
                .map(|t| t.confidence)
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(0.0);

            symbol_coverage.push(SymbolCoverage {
                name,
                kind,
                is_public,
                is_tested,
                test_count: tests.len(),
                tests,
                coverage_confidence: max_confidence,
            });
        }

        let coverage_percentage = if total_symbols > 0 {
            (tested_symbols as f64 / total_symbols as f64) * 100.0
        } else {
            0.0
        };

        Ok(FileCoverage {
            file: file_path.to_string(),
            summary: CoverageSummary {
                total_symbols,
                tested_symbols,
                untested_symbols: total_symbols - tested_symbols,
                coverage_percentage,
                test_count: 0, // Will be filled by caller
                by_type: TestTypeBreakdown::default(),
            },
            symbols: symbol_coverage,
        })
    }

    /// Get all tests that cover a specific symbol
    async fn get_symbol_tests(&self, symbol_id: i64) -> Result<Vec<TestReference>> {
        let conn = self.store.conn().await?;
        
        let mut stmt = conn.prepare(
            "SELECT 
                tf.test_name,
                f.path,
                s.start_line,
                tcm.coverage_type,
                tcm.confidence,
                tcm.call_chain
             FROM test_coverage_map tcm
             JOIN test_functions tf ON tcm.test_id = tf.id
             JOIN symbols s ON tf.symbol_id = s.id
             JOIN files f ON s.file_id = f.id
             WHERE tcm.covered_symbol_id = ?
             AND tcm.confidence >= ?
             ORDER BY tcm.confidence DESC"
        )?;

        let tests = stmt.query_map([symbol_id, self.min_confidence as i64], |row| {
            let call_chain_json: Option<String> = row.get(5)?;
            let call_chain = call_chain_json
                .and_then(|json| serde_json::from_str(&json).ok());

            Ok(TestReference {
                test_name: row.get(0)?,
                test_file: row.get(1)?,
                test_line: row.get(2)?,
                coverage_type: row.get::<_, String>(3)?.into(),
                confidence: row.get(4)?,
                call_chain,
            })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(tests)
    }

    /// Calculate project-wide coverage
    pub async fn calculate_project_coverage(
        &self,
        include_private: bool,
    ) -> Result<ProjectCoverage> {
        let conn = self.store.conn().await?;

        // Get all files with symbols
        let file_paths: Vec<String> = conn.prepare(
            "SELECT DISTINCT f.path FROM files f
             JOIN symbols s ON s.file_id = f.id
             WHERE (? OR s.is_public = TRUE)"
        )?.query_map([include_private], |row| row.get(0))?
          .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut by_file = Vec::new();
        let mut total_symbols = 0;
        let mut total_tested = 0;
        let mut total_tests = 0;

        for path in file_paths {
            let file_cov = self.calculate_file_coverage(&path, include_private).await?;
            total_symbols += file_cov.summary.total_symbols;
            total_tested += file_cov.summary.tested_symbols;
            total_tests += file_cov.summary.test_count;
            by_file.push(file_cov);
        }

        let coverage_percentage = if total_symbols > 0 {
            (total_tested as f64 / total_symbols as f64) * 100.0
        } else {
            0.0
        };

        Ok(ProjectCoverage {
            summary: CoverageSummary {
                total_symbols,
                tested_symbols: total_tested,
                untested_symbols: total_symbols - total_tested,
                coverage_percentage,
                test_count: total_tests,
                by_type: self.get_test_type_breakdown().await?,
            },
            by_file,
        })
    }

    async fn get_test_type_breakdown(&self) -> Result<TestTypeBreakdown> {
        let conn = self.store.conn().await?;
        
        let mut stmt = conn.prepare(
            "SELECT test_type, COUNT(*) FROM test_functions GROUP BY test_type"
        )?;
        
        let mut breakdown = TestTypeBreakdown::default();
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        for row in rows {
            let (test_type, count) = row?;
            match test_type.as_str() {
                "unit" => breakdown.unit_tests = count as usize,
                "integration" => breakdown.integration_tests = count as usize,
                "e2e" => breakdown.e2e_tests = count as usize,
                "benchmark" => breakdown.benchmarks = count as usize,
                _ => {}
            }
        }

        Ok(breakdown)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileCoverage {
    pub file: String,
    pub summary: CoverageSummary,
    pub symbols: Vec<SymbolCoverage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectCoverage {
    pub summary: CoverageSummary,
    pub by_file: Vec<FileCoverage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageSummary {
    pub total_symbols: usize,
    pub tested_symbols: usize,
    pub untested_symbols: usize,
    pub coverage_percentage: f64,
    pub test_count: usize,
    pub by_type: TestTypeBreakdown,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TestTypeBreakdown {
    pub unit_tests: usize,
    pub integration_tests: usize,
    pub e2e_tests: usize,
    pub benchmarks: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SymbolCoverage {
    pub name: String,
    pub kind: String,
    pub is_public: bool,
    pub is_tested: bool,
    pub test_count: usize,
    pub tests: Vec<TestReference>,
    pub coverage_confidence: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReference {
    pub test_name: String,
    pub test_file: String,
    pub test_line: usize,
    pub coverage_type: String,
    pub confidence: f64,
    pub call_chain: Option<Vec<String>>,
}

impl From<String> for CoverageType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "direct_call" => CoverageType::DirectCall,
            "indirect_call" => CoverageType::IndirectCall,
            "import_only" => CoverageType::ImportOnly,
            _ => CoverageType::DirectCall,
        }
    }
}
```

### Component 4: MCP Tool Handler

```rust
// src/tools/test_coverage.rs

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::coverage::{CoverageCalculator, TestFinder, CallTracer};
use crate::storage::Store;

#[derive(Debug, Deserialize)]
pub struct GetTestCoverageParams {
    pub file: Option<String>,
    pub module: Option<String>,
    pub project: Option<bool>,
    pub include_private: Option<bool>,
    pub min_confidence: Option<f64>,
    pub show_test_details: Option<bool>,
    pub show_untested: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TestCoverageResponse {
    pub target: Target,
    pub summary: CoverageSummary,
    pub by_file: Option<Vec<FileCoverage>>,
    pub untested_symbols: Option<Vec<UntestedSymbol>>,
    pub test_files: Vec<TestFileInfo>,
    pub computed_at: String,
}

pub async fn get_test_coverage(
    params: GetTestCoverageParams,
    store: Store,
) -> Result<TestCoverageResponse> {
    let include_private = params.include_private.unwrap_or(false);
    let min_confidence = params.min_confidence.unwrap_or(0.5);
    let show_test_details = params.show_test_details.unwrap_or(false);
    let show_untested = params.show_untested.unwrap_or(true);

    let calculator = CoverageCalculator::new(store.clone(), min_confidence);

    let (target, summary, by_file, untested) = if let Some(file) = params.file {
        // File-level coverage
        let cov = calculator.calculate_file_coverage(&file, include_private).await?;
        let untested = if show_untested {
            Some(extract_untested_symbols(&cov))
        } else {
            None
        };

        (
            Target { target_type: "file".into(), path: file },
            cov.summary,
            None,
            untested,
        )
    } else if let Some(_module) = params.module {
        // Module-level coverage (TODO: implement module resolution)
        unimplemented!("Module-level coverage not yet implemented")
    } else {
        // Project-wide coverage
        let cov = calculator.calculate_project_coverage(include_private).await?;
        let untested = if show_untested {
            Some(extract_untested_from_project(&cov))
        } else {
            None
        };

        (
            Target { target_type: "project".into(), path: ".".into() },
            cov.summary,
            Some(cov.by_file),
            untested,
        )
    };

    let test_files = get_test_file_info(&store).await?;

    Ok(TestCoverageResponse {
        target,
        summary,
        by_file,
        untested_symbols: untested,
        test_files,
        computed_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn extract_untested_symbols(cov: &FileCoverage) -> Vec<UntestedSymbol> {
    cov.symbols.iter()
        .filter(|s| !s.is_tested && s.is_public)
        .map(|s| UntestedSymbol {
            name: s.name.clone(),
            kind: s.kind.clone(),
            file: cov.file.clone(),
            line: 0, // Would need to query
            is_public: s.is_public,
            suggestion: Some(format!("test_{}", s.name.to_lowercase())),
        })
        .collect()
}

fn extract_untested_from_project(cov: &ProjectCoverage) -> Vec<UntestedSymbol> {
    cov.by_file.iter()
        .flat_map(|f| extract_untested_symbols(f))
        .collect()
}

async fn get_test_file_info(store: &Store) -> Result<Vec<TestFileInfo>> {
    let conn = store.conn().await?;
    
    let mut stmt = conn.prepare(
        "SELECT 
            f.path,
            COUNT(tf.id) as test_count,
            COUNT(DISTINCT tcm.covered_symbol_id) as coverage_contribution
         FROM files f
         JOIN test_functions tf ON tf.file_id = f.id
         LEFT JOIN test_coverage_map tcm ON tcm.test_id = tf.id
         GROUP BY f.id
         ORDER BY coverage_contribution DESC"
    )?;

    let test_files = stmt.query_map([], |row| {
        Ok(TestFileInfo {
            path: row.get(0)?,
            test_count: row.get(1)?,
            coverage_contribution: row.get(2)?,
        })
    })?.collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(test_files)
}

#[derive(Debug, Serialize)]
struct Target {
    target_type: String,
    path: String,
}

#[derive(Debug, Serialize)]
pub struct UntestedSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub is_public: bool,
    pub suggestion: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestFileInfo {
    pub path: String,
    pub test_count: usize,
    pub coverage_contribution: usize,
}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_find_rust_tests() {
        let finder = TestFinder::new().unwrap();
        let content = b"
            #[test]
            fn test_addition() {
                assert_eq!(2 + 2, 4);
            }

            #[tokio::test]
            async fn test_async_operation() {
                let result = async_fn().await;
                assert!(result.is_ok());
            }
        ";

        let file = File {
            id: 1,
            path: "test.rs".into(),
            ..Default::default()
        };

        let tests = finder.find_tests(&file, content).unwrap();
        assert_eq!(tests.len(), 2);
        assert_eq!(tests[0].name, "test_addition");
        assert!(!tests[0].is_async);
        assert_eq!(tests[1].name, "test_async_operation");
        assert!(tests[1].is_async);
    }

    #[tokio::test]
    async fn test_call_tracer_direct_call() {
        let store = create_test_store().await;
        let tracer = CallTracer::new(store.clone());

        // Setup: test_fn -> target_fn
        let test_id = insert_test_function(&store, "test_fn").await;
        let target_id = insert_function(&store, "target_fn").await;
        insert_call_reference(&store, test_id, target_id).await;

        let coverage = tracer.trace_coverage(test_id).await.unwrap();
        
        assert!(coverage.contains_key(&target_id));
        assert_eq!(coverage[&target_id].confidence, 1.0);
        assert_eq!(coverage[&target_id].coverage_type, CoverageType::DirectCall);
    }

    #[tokio::test]
    async fn test_call_tracer_indirect_call() {
        let store = create_test_store().await;
        let tracer = CallTracer::new(store.clone());

        // Setup: test_fn -> helper_fn -> target_fn
        let test_id = insert_test_function(&store, "test_fn").await;
        let helper_id = insert_function(&store, "helper_fn").await;
        let target_id = insert_function(&store, "target_fn").await;
        insert_call_reference(&store, test_id, helper_id).await;
        insert_call_reference(&store, helper_id, target_id).await;

        let coverage = tracer.trace_coverage(test_id).await.unwrap();
        
        assert!(coverage.contains_key(&target_id));
        assert_eq!(coverage[&target_id].confidence, 0.8); // 1 hop
        assert_eq!(coverage[&target_id].coverage_type, CoverageType::IndirectCall);
        assert_eq!(coverage[&target_id].call_chain.len(), 3);
    }

    #[tokio::test]
    async fn test_coverage_calculator_file() {
        let store = create_test_store().await;
        populate_test_data(&store).await;

        let calculator = CoverageCalculator::new(store, 0.5);
        let coverage = calculator.calculate_file_coverage("src/api.rs", false).await.unwrap();

        assert_eq!(coverage.summary.total_symbols, 5);
        assert_eq!(coverage.summary.tested_symbols, 3);
        assert_eq!(coverage.summary.coverage_percentage, 60.0);
        assert_eq!(coverage.symbols.len(), 5);
    }
}
```

### Integration Tests

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_end_to_end_coverage() {
        // Create temporary project
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("src");
        std::fs::create_dir_all(&src_path).unwrap();

        // Write source file
        std::fs::write(
            src_path.join("lib.rs"),
            r#"
                pub fn add(a: i32, b: i32) -> i32 { a + b }
                pub fn multiply(a: i32, b: i32) -> i32 { a * b }
                pub fn divide(a: i32, b: i32) -> Option<i32> {
                    if b != 0 { Some(a / b) } else { None }
                }
            "#
        ).unwrap();

        // Write test file
        std::fs::write(
            src_path.join("tests.rs"),
            r#"
                use super::*;

                #[test]
                fn test_add() {
                    assert_eq!(add(2, 2), 4);
                }

                #[test]
                fn test_multiply() {
                    assert_eq!(multiply(3, 4), 12);
                }
                // No test for divide!
            "#
        ).unwrap();

        // Index project
        let store = Store::new(temp.path()).await.unwrap();
        let indexer = Indexer::new(store.clone());
        indexer.index_directory(temp.path()).await.unwrap();

        // Get coverage
        let params = GetTestCoverageParams {
            project: Some(true),
            include_private: Some(false),
            show_untested: Some(true),
            ..Default::default()
        };

        let response = get_test_coverage(params, store).await.unwrap();

        assert_eq!(response.summary.total_symbols, 3);
        assert_eq!(response.summary.tested_symbols, 2);
        assert_eq!(response.summary.untested_symbols, 1);
        assert!((response.summary.coverage_percentage - 66.67).abs() < 0.1);

        let untested = response.untested_symbols.unwrap();
        assert_eq!(untested.len(), 1);
        assert_eq!(untested[0].name, "divide");
    }
}
```

---

## Usage Examples

### Example 1: Check File Coverage

```bash
# Find which functions in users.rs have tests
mcp query get_test_coverage '{"file": "src/api/users.rs", "show_test_details": true}'
```

**Response**:
```json
{
  "target": {"type": "file", "path": "src/api/users.rs"},
  "summary": {
    "total_symbols": 8,
    "tested_symbols": 5,
    "untested_symbols": 3,
    "coverage_percentage": 62.5,
    "test_count": 7,
    "by_type": {
      "unit_tests": 5,
      "integration_tests": 2,
      "e2e_tests": 0,
      "benchmarks": 0
    }
  },
  "untested_symbols": [
    {
      "name": "delete_user",
      "kind": "function",
      "file": "src/api/users.rs",
      "line": 42,
      "is_public": true,
      "suggestion": "test_delete_user"
    },
    {
      "name": "update_email",
      "kind": "function",
      "file": "src/api/users.rs",
      "line": 58,
      "is_public": true,
      "suggestion": "test_update_email"
    }
  ],
  "test_files": [
    {
      "path": "tests/api/users_test.rs",
      "test_count": 5,
      "coverage_contribution": 5
    },
    {
      "path": "tests/integration/user_flow_test.rs",
      "test_count": 2,
      "coverage_contribution": 3
    }
  ],
  "computed_at": "2026-02-16T10:30:00Z"
}
```

### Example 2: Project-Wide Coverage

```bash
mcp query get_test_coverage '{"project": true, "min_confidence": 0.6}'
```

**Response**:
```json
{
  "target": {"type": "project", "path": "."},
  "summary": {
    "total_symbols": 247,
    "tested_symbols": 189,
    "untested_symbols": 58,
    "coverage_percentage": 76.5,
    "test_count": 142,
    "by_type": {
      "unit_tests": 98,
      "integration_tests": 32,
      "e2e_tests": 8,
      "benchmarks": 4
    }
  },
  "by_file": [
    {
      "file": "src/api/users.rs",
      "summary": {"total_symbols": 8, "tested_symbols": 5, "coverage_percentage": 62.5}
    },
    {
      "file": "src/storage/sqlite.rs",
      "summary": {"total_symbols": 24, "tested_symbols": 22, "coverage_percentage": 91.7}
    }
    // ... more files
  ],
  "test_files": [
    {"path": "tests/api_test.rs", "test_count": 45, "coverage_contribution": 67},
    {"path": "tests/storage_test.rs", "test_count": 38, "coverage_contribution": 52}
    // ... more test files
  ],
  "computed_at": "2026-02-16T10:35:00Z"
}
```

### Example 3: Identify Untested Critical Functions

```bash
# Find all untested public functions in authentication module
mcp query get_test_coverage '{
  "file": "src/auth/mod.rs",
  "include_private": false,
  "show_untested": true
}'
```

---

## Performance Metrics

### Targets

- **File coverage**: < 2 seconds
- **Module coverage**: < 5 seconds
- **Project coverage**: < 10 seconds (for 500 files)
- **Memory**: < 100MB additional overhead

### Optimization Strategies

1. **Caching**: Cache coverage results, invalidate on file changes
2. **Incremental**: Only recalculate coverage for changed files
3. **Indexing**: Proper indexes on `test_coverage_map`
4. **Lazy loading**: Don't load test details unless requested
5. **Parallel**: Calculate file coverage in parallel for project-wide

---

## Success Metrics

### Key Performance Indicators (KPIs)

1. **Accuracy**: > 95% correct test-to-source mapping
2. **Performance**: < 5s for project-wide coverage
3. **Coverage tracking**: Able to track coverage trends over time
4. **Developer adoption**: Used by > 80% of team

### Acceptance Criteria

- [ ] Correctly identifies all test functions in Rust, TypeScript, Python
- [ ] Accurately traces direct and indirect function calls (up to 3 hops)
- [ ] Calculates coverage percentages at file, module, project levels
- [ ] Identifies untested public functions with < 5% false positives
- [ ] Provides actionable suggestions for missing tests
- [ ] Responds within performance targets
- [ ] Works correctly with async tests
- [ ] Handles edge cases: macros, trait impls, generic functions

---

## Future Enhancements

1. **Runtime coverage integration**: Merge with `cargo tarpaulin` results
2. **Coverage trends**: Track coverage over time, show regressions
3. **Critical path coverage**: Identify untested critical code paths
4. **Test quality scoring**: Analyze assertion strength, edge case coverage
5. **Auto-generate tests**: Suggest test scaffolding for untested functions
6. **Coverage gates**: CI integration to enforce minimum coverage
7. **Visual coverage reports**: HTML reports with line-by-line highlighting
8. **Mutation testing**: Detect weak tests that don't catch bugs

---

## References

- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [cargo-tarpaulin](https://github.com/xd009642/tarpaulin) - Code coverage tool
- [Jest Coverage](https://jestjs.io/docs/configuration#collectcoverage-boolean) - JS coverage
- [pytest-cov](https://pytest-cov.readthedocs.io/) - Python coverage
- Tree-sitter queries for test detection
