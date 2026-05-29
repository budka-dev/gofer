# Feature: Lightweight Existence Checks - Минимальные токены для проверок

**ID:** PHASE0-005  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)

---

## 📋 Описание

Набор MCP tools для быстрых boolean/existence проверок без полного чтения файлов. Экономит 95% токенов для простых вопросов типа "существует ли файл?", "есть ли тесты?", "экспортирован ли символ?".

### Проблема

**Текущий workflow для простых вопросов:**

```
Q: "Does file src/auth.rs exist?"
A: Calls read_file("src/auth.rs") → 6000 tokens → "Yes, it exists"

Q: "Is function `verify_token` exported?"  
A: Calls read_file("src/auth.rs") → 6000 tokens → parses → "Yes, it's pub"

Q: "Are there tests for MyStruct?"
A: Calls search("test MyStruct") → 5000 tokens → "Yes, 3 tests found"
```

**Проблемы:**
- Тратим 5000-6000 токенов на boolean ответ
- Медленно (full read/search vs simple query)
- Неэффективно для batch checks
- AI генерирует 95% ненужного контекста

### Решение

Lightweight tools для existence checks:

```
Q: "Does file src/auth.rs exist?"
A: Calls file_exists("src/auth.rs") → 0 tokens → true

Q: "Is function `verify_token` exported?"  
A: Calls is_exported("verify_token") → 0 tokens → { exists: true, exported: true }

Q: "Are there tests for MyStruct?"
A: Calls has_tests_for("MyStruct") → 0 tokens → { has_tests: true, count: 3 }
```

**Savings:** 95-100% токенов для existence checks

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ 95%+ экономия токенов для existence checks
- ✅ < 50ms response time (simple SQL queries)
- ✅ Cover common check scenarios
- ✅ Enable efficient batch operations

### Non-Goals
- ❌ Не возвращает полный контекст (только boolean/count)
- ❌ Не делает complex analysis (просто проверка существования)
- ❌ Не заменяет полное чтение (для details нужен read_file)

---

## 🏗️ Архитектура

### Tools Overview

```
┌──────────────────────────────────────────────┐
│         Lightweight Check Tools              │
├──────────────────────────────────────────────┤
│  file_exists()        symbol_exists()        │
│  has_tests_for()      has_documentation()    │
│  is_exported()        get_symbol_location()  │
└────────────────┬─────────────────────────────┘
                 │
        ┌────────▼────────┐
        │  SQLite Storage │
        │  (Fast Queries) │
        └─────────────────┘
```

All tools use simple, indexed SQL queries → Fast & cheap

---

## 🔧 API Specification

### 1. file_exists

```json
{
  "name": "file_exists",
  "description": "Check if file exists in index (fast boolean check, no content returned)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "File path (relative to project root)"
      }
    },
    "required": ["path"]
  }
}
```

**Response:**
```rust
#[derive(Serialize)]
pub struct FileExistsResponse {
    pub exists: bool,
    pub path: String,
    pub indexed: bool,  // true if exists AND indexed
}
```

**Example:**
```json
{
  "exists": true,
  "path": "src/auth.rs",
  "indexed": true
}
```

---

### 2. symbol_exists

```json
{
  "name": "symbol_exists",
  "description": "Check if symbol exists in index. Returns location if found.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": {
        "type": "string",
        "description": "Symbol name to search for"
      },
      "kind": {
        "type": "string",
        "enum": ["function", "struct", "enum", "trait", "class", "interface"],
        "description": "Optional: filter by symbol kind"
      },
      "file": {
        "type": "string",
        "description": "Optional: limit search to specific file"
      }
    },
    "required": ["name"]
  }
}
```

**Response:**
```rust
#[derive(Serialize)]
pub struct SymbolExistsResponse {
    pub exists: bool,
    pub name: String,
    pub locations: Vec<SymbolLocation>,  // Can be multiple (overloads, etc)
}

#[derive(Serialize)]
pub struct SymbolLocation {
    pub file: String,
    pub line: u32,
    pub kind: String,
    pub visibility: String,  // "public", "private", "internal"
}
```

**Example:**
```json
{
  "exists": true,
  "name": "verify_token",
  "locations": [
    {
      "file": "src/auth.rs",
      "line": 45,
      "kind": "function",
      "visibility": "public"
    }
  ]
}
```

---

### 3. has_tests_for

```json
{
  "name": "has_tests_for",
  "description": "Check if tests exist for a symbol/module. Returns count and test locations.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "target": {
        "type": "string",
        "description": "Symbol name or module to check tests for"
      },
      "kind": {
        "type": "string",
        "enum": ["symbol", "module", "file"],
        "default": "symbol",
        "description": "What kind of target"
      }
    },
    "required": ["target"]
  }
}
```

**Response:**
```rust
#[derive(Serialize)]
pub struct HasTestsResponse {
    pub has_tests: bool,
    pub target: String,
    pub test_count: usize,
    pub test_locations: Vec<TestLocation>,
    pub coverage_estimate: Option<f32>,  // If coverage data available
}

#[derive(Serialize)]
pub struct TestLocation {
    pub file: String,
    pub line: u32,
    pub test_name: String,
    pub test_type: String,  // "unit", "integration", "doc"
}
```

**Example:**
```json
{
  "has_tests": true,
  "target": "MyStruct",
  "test_count": 5,
  "test_locations": [
    {
      "file": "src/mystruct.rs",
      "line": 120,
      "test_name": "test_new",
      "test_type": "unit"
    },
    {
      "file": "tests/integration_test.rs",
      "line": 34,
      "test_name": "test_mystruct_creation",
      "test_type": "integration"
    }
  ],
  "coverage_estimate": 85.5
}
```

---

### 4. has_documentation

```json
{
  "name": "has_documentation",
  "description": "Check if symbol has documentation (doc comments)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol": {
        "type": "string",
        "description": "Symbol name to check"
      },
      "file": {
        "type": "string",
        "description": "Optional: specific file to check"
      }
    },
    "required": ["symbol"]
  }
}
```

**Response:**
```rust
#[derive(Serialize)]
pub struct HasDocumentationResponse {
    pub has_docs: bool,
    pub symbol: String,
    pub doc_summary: Option<String>,  // First line of doc comment
    pub doc_length: usize,  // Characters in doc comment
    pub examples_count: usize,  // Number of code examples in docs
}
```

**Example:**
```json
{
  "has_docs": true,
  "symbol": "verify_token",
  "doc_summary": "Verifies JWT token signature and expiration",
  "doc_length": 245,
  "examples_count": 2
}
```

---

### 5. is_exported

```json
{
  "name": "is_exported",
  "description": "Check if symbol is public/exported",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol": {
        "type": "string",
        "description": "Symbol name to check"
      },
      "file": {
        "type": "string",
        "description": "Optional: specific file to check"
      }
    },
    "required": ["symbol"]
  }
}
```

**Response:**
```rust
#[derive(Serialize)]
pub struct IsExportedResponse {
    pub exists: bool,
    pub symbol: String,
    pub exported: bool,
    pub visibility: String,  // "public", "private", "internal", "protected"
    pub export_path: Option<String>,  // Module path for export
}
```

**Example:**
```json
{
  "exists": true,
  "symbol": "verify_token",
  "exported": true,
  "visibility": "public",
  "export_path": "crate::auth::verify_token"
}
```

---

### 6. get_symbol_location (lightweight)

```json
{
  "name": "get_symbol_location",
  "description": "Get file and line number for symbol (no content)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol": {
        "type": "string",
        "description": "Symbol name"
      },
      "kind": {
        "type": "string",
        "description": "Optional: filter by kind"
      }
    },
    "required": ["symbol"]
  }
}
```

**Response:**
```rust
#[derive(Serialize)]
pub struct SymbolLocationResponse {
    pub found: bool,
    pub symbol: String,
    pub locations: Vec<LocationInfo>,
}

#[derive(Serialize)]
pub struct LocationInfo {
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
    pub kind: String,
}
```

---

## 💻 Implementation Details

### 1. file_exists

```rust
// src/daemon/tools/lightweight_checks.rs

pub async fn handle_file_exists(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
) -> Result<Value> {
    let path = args.get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing path"))?;
    
    // Fast SQL query with index
    let exists = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM files WHERE path = ?",
        path
    )
    .fetch_one(&sqlite.pool)
    .await? > 0;
    
    let indexed = if exists {
        sqlx::query_scalar!(
            "SELECT index_status FROM files WHERE path = ?",
            path
        )
        .fetch_optional(&sqlite.pool)
        .await?
        .map(|status| status == "completed")
        .unwrap_or(false)
    } else {
        false
    };
    
    let response = FileExistsResponse {
        exists,
        path: path.to_string(),
        indexed,
    };
    
    Ok(serde_json::to_value(response)?)
}
```

**SQL Performance:**
- Uses index: `CREATE INDEX idx_files_path ON files(path)`
- Query time: < 1ms
- No full table scan

---

### 2. symbol_exists

```rust
pub async fn handle_symbol_exists(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
) -> Result<Value> {
    let name = args.get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing name"))?;
    
    let kind = args.get("kind")
        .and_then(|v| v.as_str());
    
    let file = args.get("file")
        .and_then(|v| v.as_str());
    
    // Build query dynamically
    let mut query = String::from(
        "SELECT s.name, s.kind, s.line, s.visibility, f.path as file
         FROM symbols s
         JOIN files f ON s.file_id = f.id
         WHERE s.name = ?"
    );
    
    let mut params: Vec<&str> = vec![name];
    
    if let Some(k) = kind {
        query.push_str(" AND s.kind = ?");
        params.push(k);
    }
    
    if let Some(f) = file {
        query.push_str(" AND f.path = ?");
        params.push(f);
    }
    
    query.push_str(" LIMIT 10");  // Prevent huge results
    
    // Execute
    let locations = sqlx::query_as::<_, SymbolRow>(&query)
        .bind(name)
        .fetch_all(&sqlite.pool)
        .await?;
    
    let exists = !locations.is_empty();
    
    let response = SymbolExistsResponse {
        exists,
        name: name.to_string(),
        locations: locations.into_iter().map(|row| SymbolLocation {
            file: row.file,
            line: row.line as u32,
            kind: row.kind,
            visibility: row.visibility.unwrap_or_else(|| "unknown".to_string()),
        }).collect(),
    };
    
    Ok(serde_json::to_value(response)?)
}

#[derive(sqlx::FromRow)]
struct SymbolRow {
    name: String,
    kind: String,
    line: i64,
    visibility: Option<String>,
    file: String,
}
```

**SQL Performance:**
- Uses index: `CREATE INDEX idx_symbols_name ON symbols(name)`
- Query time: < 5ms
- Limit results to prevent abuse

---

### 3. has_tests_for

```rust
pub async fn handle_has_tests_for(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
) -> Result<Value> {
    let target = args.get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing target"))?;
    
    let kind = args.get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("symbol");
    
    let test_locations = match kind {
        "symbol" => find_symbol_tests(sqlite, target).await?,
        "module" => find_module_tests(sqlite, target).await?,
        "file" => find_file_tests(sqlite, target).await?,
        _ => return Err(anyhow!("Invalid kind: {}", kind)),
    };
    
    let has_tests = !test_locations.is_empty();
    let test_count = test_locations.len();
    
    // Try to get coverage estimate (if coverage data available)
    let coverage_estimate = get_coverage_for_target(sqlite, target).await.ok();
    
    let response = HasTestsResponse {
        has_tests,
        target: target.to_string(),
        test_count,
        test_locations,
        coverage_estimate,
    };
    
    Ok(serde_json::to_value(response)?)
}

async fn find_symbol_tests(
    sqlite: &SqliteStorage,
    symbol_name: &str,
) -> Result<Vec<TestLocation>> {
    // Strategy 1: Look for test functions with name pattern
    let test_pattern_1 = format!("test_{}", symbol_name.to_lowercase());
    let test_pattern_2 = format!("{}_test", symbol_name.to_lowercase());
    
    let rows = sqlx::query!(
        r#"
        SELECT f.path, s.line, s.name
        FROM symbols s
        JOIN files f ON s.file_id = f.id
        WHERE s.kind = 'function'
          AND (
            s.name LIKE ? OR 
            s.name LIKE ? OR
            s.name LIKE ?
          )
        "#,
        test_pattern_1,
        test_pattern_2,
        format!("test%{}%", symbol_name.to_lowercase())
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    // Strategy 2: Look in test files that reference the symbol
    let test_refs = sqlx::query!(
        r#"
        SELECT DISTINCT f.path, s.line, s.name
        FROM symbols s
        JOIN files f ON s.file_id = f.id
        WHERE f.path LIKE '%test%'
          AND s.kind = 'function'
          AND EXISTS (
            SELECT 1 FROM references r
            WHERE r.symbol_name = ?
              AND r.file_id = f.id
          )
        "#,
        symbol_name
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    let mut locations = Vec::new();
    
    // Combine results
    for row in rows.iter().chain(test_refs.iter()) {
        locations.push(TestLocation {
            file: row.path.clone(),
            line: row.line as u32,
            test_name: row.name.clone(),
            test_type: infer_test_type(&row.path),
        });
    }
    
    // Deduplicate
    locations.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    locations.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    
    Ok(locations)
}

async fn find_module_tests(
    sqlite: &SqliteStorage,
    module_path: &str,
) -> Result<Vec<TestLocation>> {
    // Find test files for module
    let test_pattern = format!("{}%test%", module_path);
    
    let rows = sqlx::query!(
        r#"
        SELECT f.path, s.line, s.name
        FROM symbols s
        JOIN files f ON s.file_id = f.id
        WHERE f.path LIKE ?
          AND s.kind = 'function'
          AND (s.name LIKE 'test_%' OR f.path LIKE '%_test.%')
        "#,
        test_pattern
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    Ok(rows.into_iter().map(|row| TestLocation {
        file: row.path,
        line: row.line as u32,
        test_name: row.name,
        test_type: "unit".to_string(),
    }).collect())
}

async fn find_file_tests(
    sqlite: &SqliteStorage,
    file_path: &str,
) -> Result<Vec<TestLocation>> {
    // Find test functions in same file or corresponding test file
    let rows = sqlx::query!(
        r#"
        SELECT s.line, s.name
        FROM symbols s
        JOIN files f ON s.file_id = f.id
        WHERE f.path = ?
          AND s.kind = 'function'
          AND s.name LIKE 'test_%'
        "#,
        file_path
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    let mut locations: Vec<TestLocation> = rows.into_iter().map(|row| TestLocation {
        file: file_path.to_string(),
        line: row.line as u32,
        test_name: row.name,
        test_type: "unit".to_string(),
    }).collect();
    
    // Also check corresponding test file
    let test_file_path = get_test_file_path(file_path);
    if test_file_path != file_path {
        let test_rows = sqlx::query!(
            r#"
            SELECT s.line, s.name
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE f.path = ?
              AND s.kind = 'function'
            "#,
            test_file_path
        )
        .fetch_all(&sqlite.pool)
        .await
        .unwrap_or_default();
        
        locations.extend(test_rows.into_iter().map(|row| TestLocation {
            file: test_file_path.clone(),
            line: row.line as u32,
            test_name: row.name,
            test_type: "unit".to_string(),
        }));
    }
    
    Ok(locations)
}

fn get_test_file_path(file_path: &str) -> String {
    // src/auth.rs → tests/auth_test.rs or src/auth_test.rs
    if file_path.contains("tests/") {
        return file_path.to_string();
    }
    
    let stem = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    
    // Try common patterns
    vec![
        format!("tests/{}_test.{}", stem, ext),
        format!("src/{}_test.{}", stem, ext),
        format!("{}s/test_{}.{}", file_path.split('/').next().unwrap_or(""), stem, ext),
    ]
    .into_iter()
    .next()
    .unwrap_or_else(|| file_path.to_string())
}

fn infer_test_type(path: &str) -> String {
    if path.contains("tests/") {
        "integration".to_string()
    } else if path.contains("_test") || path.contains("test_") {
        "unit".to_string()
    } else {
        "unknown".to_string()
    }
}

async fn get_coverage_for_target(
    sqlite: &SqliteStorage,
    target: &str,
) -> Result<f32> {
    // Query coverage data (if available from previous coverage run)
    // This is optional - return None if no coverage data
    
    // Placeholder: would integrate with coverage tools
    Ok(0.0)
}
```

---

### 4. has_documentation

```rust
pub async fn handle_has_documentation(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
) -> Result<Value> {
    let symbol = args.get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing symbol"))?;
    
    let file = args.get("file")
        .and_then(|v| v.as_str());
    
    // Query for symbol with doc comment
    let row = if let Some(f) = file {
        sqlx::query!(
            r#"
            SELECT s.doc_comment
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE s.name = ? AND f.path = ?
            LIMIT 1
            "#,
            symbol,
            f
        )
        .fetch_optional(&sqlite.pool)
        .await?
    } else {
        sqlx::query!(
            r#"
            SELECT s.doc_comment
            FROM symbols s
            WHERE s.name = ?
            LIMIT 1
            "#,
            symbol
        )
        .fetch_optional(&sqlite.pool)
        .await?
    };
    
    let (has_docs, doc_summary, doc_length, examples_count) = if let Some(row) = row {
        if let Some(doc) = row.doc_comment {
            let summary = doc.lines().next().map(|s| s.to_string());
            let length = doc.len();
            let examples = doc.matches("```").count() / 2;  // Pairs of ```
            
            (true, summary, length, examples)
        } else {
            (false, None, 0, 0)
        }
    } else {
        (false, None, 0, 0)
    };
    
    let response = HasDocumentationResponse {
        has_docs,
        symbol: symbol.to_string(),
        doc_summary,
        doc_length,
        examples_count,
    };
    
    Ok(serde_json::to_value(response)?)
}
```

---

### 5. is_exported

```rust
pub async fn handle_is_exported(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
) -> Result<Value> {
    let symbol = args.get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing symbol"))?;
    
    let file = args.get("file")
        .and_then(|v| v.as_str());
    
    let row = if let Some(f) = file {
        sqlx::query!(
            r#"
            SELECT s.visibility, s.export_path
            FROM symbols s
            JOIN files f ON s.file_id = f.id
            WHERE s.name = ? AND f.path = ?
            LIMIT 1
            "#,
            symbol,
            f
        )
        .fetch_optional(&sqlite.pool)
        .await?
    } else {
        sqlx::query!(
            r#"
            SELECT s.visibility, s.export_path
            FROM symbols s
            WHERE s.name = ?
            ORDER BY CASE 
                WHEN s.visibility = 'public' THEN 1
                ELSE 2
            END
            LIMIT 1
            "#,
            symbol
        )
        .fetch_optional(&sqlite.pool)
        .await?
    };
    
    let (exists, exported, visibility, export_path) = if let Some(row) = row {
        let vis = row.visibility.unwrap_or_else(|| "unknown".to_string());
        let is_exported = vis == "public" || vis == "exported";
        
        (true, is_exported, vis, row.export_path)
    } else {
        (false, false, "unknown".to_string(), None)
    };
    
    let response = IsExportedResponse {
        exists,
        symbol: symbol.to_string(),
        exported,
        visibility,
        export_path,
    };
    
    Ok(serde_json::to_value(response)?)
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_file_exists() {
    let sqlite = setup_test_db().await;
    
    // Insert test file
    sqlx::query!("INSERT INTO files (path, index_status) VALUES ('test.rs', 'completed')")
        .execute(&sqlite.pool)
        .await
        .unwrap();
    
    let mut args = Map::new();
    args.insert("path".into(), Value::String("test.rs".into()));
    
    let response = handle_file_exists(&args, &sqlite).await.unwrap();
    let resp: FileExistsResponse = serde_json::from_value(response).unwrap();
    
    assert!(resp.exists);
    assert!(resp.indexed);
}

#[tokio::test]
async fn test_symbol_exists() {
    let sqlite = setup_test_db().await;
    
    // Setup: insert file and symbol
    let file_id = insert_test_file(&sqlite, "test.rs").await;
    insert_test_symbol(&sqlite, file_id, "my_function", "function", 10).await;
    
    let mut args = Map::new();
    args.insert("name".into(), Value::String("my_function".into()));
    
    let response = handle_symbol_exists(&args, &sqlite).await.unwrap();
    let resp: SymbolExistsResponse = serde_json::from_value(response).unwrap();
    
    assert!(resp.exists);
    assert_eq!(resp.locations.len(), 1);
    assert_eq!(resp.locations[0].file, "test.rs");
    assert_eq!(resp.locations[0].line, 10);
}

#[tokio::test]
async fn test_has_tests_for() {
    let sqlite = setup_test_db().await;
    
    // Setup: insert symbol and test
    let file_id = insert_test_file(&sqlite, "src/auth.rs").await;
    insert_test_symbol(&sqlite, file_id, "verify_token", "function", 45).await;
    
    let test_file_id = insert_test_file(&sqlite, "src/auth.rs").await;
    insert_test_symbol(&sqlite, test_file_id, "test_verify_token", "function", 120).await;
    
    let mut args = Map::new();
    args.insert("target".into(), Value::String("verify_token".into()));
    
    let response = handle_has_tests_for(&args, &sqlite).await.unwrap();
    let resp: HasTestsResponse = serde_json::from_value(response).unwrap();
    
    assert!(resp.has_tests);
    assert!(resp.test_count > 0);
}

#[tokio::test]
async fn test_performance() {
    let sqlite = setup_large_test_db().await;  // 10k symbols
    
    let start = Instant::now();
    
    for _ in 0..100 {
        let mut args = Map::new();
        args.insert("name".into(), Value::String("random_symbol".into()));
        handle_symbol_exists(&args, &sqlite).await.unwrap();
    }
    
    let duration = start.elapsed();
    let avg = duration.as_millis() / 100;
    
    // Should average < 10ms per query
    assert!(avg < 10, "Average query time: {}ms (expected < 10ms)", avg);
}
```

---

## 📈 Success Metrics

### Token Savings
- **Target:** 95-100% for existence checks
- **Measurement:** Compare with full read_file approach

### Performance
- ⏱️ file_exists: < 5ms
- ⏱️ symbol_exists: < 10ms
- ⏱️ has_tests_for: < 50ms (more complex)
- ⏱️ has_documentation: < 10ms
- ⏱️ is_exported: < 5ms

### Accuracy
- ✅ 100% accuracy for indexed data
- ✅ No false positives
- ⚠️ May miss data not yet indexed (acceptable)

---

## 📚 Usage Examples

### Example 1: Quick File Check

```typescript
// Before (wasteful)
try {
  const file = await gofer.read_file({ file_path: "src/auth.rs" });
  console.log("File exists");
} catch {
  console.log("File doesn't exist");
}
// Cost: 6000 tokens

// After (efficient)
const exists = await gofer.file_exists({ path: "src/auth.rs" });
console.log(exists.exists ? "File exists" : "File doesn't exist");
// Cost: 0 tokens
```

### Example 2: Batch Symbol Checks

```typescript
// Check multiple symbols efficiently
const symbols = ["verify_token", "hash_password", "generate_session"];

for (const sym of symbols) {
  const result = await gofer.symbol_exists({ name: sym });
  console.log(`${sym}: ${result.exists ? '✓' : '✗'}`);
}
// Cost: 0 tokens (vs 18000 with read_file for each)
```

### Example 3: Test Coverage Check

```typescript
// Find symbols without tests
const symbols = await gofer.get_symbols({ file: "src/auth.rs" });

for (const sym of symbols.symbols) {
  const tests = await gofer.has_tests_for({ target: sym.name });
  
  if (!tests.has_tests) {
    console.log(`⚠️  ${sym.name} has no tests`);
  }
}
```

### Example 4: API Documentation Check

```typescript
// Find exported symbols without documentation
const symbols = await gofer.get_symbols({ visibility: "public" });

const undocumented = [];

for (const sym of symbols.symbols) {
  const docs = await gofer.has_documentation({ symbol: sym.name });
  
  if (!docs.has_docs) {
    undocumented.push(sym.name);
  }
}

console.log(`${undocumented.length} public symbols lack documentation`);
```

---

## ✅ Acceptance Criteria

- [ ] All 6 tools implemented and working
- [ ] Response time < 50ms for all tools
- [ ] Token savings >= 95% vs full reads
- [ ] SQL queries use indexes
- [ ] Handles non-existent symbols gracefully
- [ ] Batch operations are efficient
- [ ] All unit tests pass
- [ ] Performance benchmarks pass
- [ ] Documentation complete

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD
