# Feature: get_symbol_context - Unified Symbol Tool

**ID:** PHASE1-019  
**Priority:** 🔥🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 1 (Optimization & Unified Tools)

---

## 📋 Описание

**Unified инструмент** заменяющий 4-6 отдельных MCP вызовов. Возвращает полный контекст символа за один запрос: definition, callers, callees, references, documentation, related tests.

### Проблема

**Текущий workflow (раздельные вызовы):**
```
AI: "Расскажи про функцию authenticate"

Вызовы:
1. search("authenticate") → find location (200ms)
2. read_file("auth.rs", lines=50-100) → get definition (150ms)
3. get_callers("authenticate") → who calls it (300ms)
4. get_callees("authenticate") → what it calls (300ms)
5. get_references("authenticate") → all usages (200ms)
6. search("test.*authenticate") → find tests (150ms)

Total: 6 requests, 1300ms, ~500 токенов на communication overhead
```

**С get_symbol_context (unified):**
```
AI: get_symbol_context("authenticate")

Total: 1 request, 400ms, ~50 токенов overhead
Speedup: 3.25× faster
Token savings: 450 токенов (90%)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Unified tool вместо 6 отдельных
- ✅ Single round-trip
- ✅ 60-70% токенов экономии
- ✅ 2-3× faster response
- ✅ Comprehensive symbol context

### Non-Goals
- ❌ Не заменяет специализированные tools полностью
- ❌ Не recursive expansion (только 1 уровень)

---

## 🏗️ Архитектура

```
┌─────────────────────────────────────────┐
│    get_symbol_context(symbol_name)      │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Symbol Locator │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬────────────┬────────────┬────────────┐
     │           │           │            │            │            │
┌────▼─────┐┌───▼────┐┌────▼──────┐┌────▼──────┐┌───▼────┐┌─────▼──────┐
│Definition││Callers ││  Callees  ││References ││  Docs  ││   Tests    │
│ Finder   ││ Finder ││  Finder   ││  Finder   ││Extractor││  Finder   │
└──────────┘└────────┘└───────────┘└───────────┘└────────┘└────────────┘
                                │
                       ┌────────▼────────┐
                       │   Aggregator    │
                       │  (parallel)     │
                       └─────────────────┘
```

---

## 📊 Data Model

### MCP Tool Definition

```json
{
  "name": "get_symbol_context",
  "description": "Получить полный контекст символа за один запрос (definition, callers, callees, references, docs, tests)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol": {
        "type": "string",
        "description": "Имя символа (функция, класс, тип)"
      },
      "include_callers": {
        "type": "boolean",
        "default": true
      },
      "include_callees": {
        "type": "boolean",
        "default": true
      },
      "include_references": {
        "type": "boolean",
        "default": true
      },
      "include_tests": {
        "type": "boolean",
        "default": true
      },
      "max_callers": {
        "type": "number",
        "default": 20
      },
      "max_callees": {
        "type": "number",
        "default": 20
      }
    },
    "required": ["symbol"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct SymbolContext {
    /// Symbol definition
    pub definition: SymbolDefinition,
    
    /// Who calls this symbol
    pub callers: Option<Vec<Caller>>,
    
    /// What this symbol calls
    pub callees: Option<Vec<Callee>>,
    
    /// All references to this symbol
    pub references: Option<Vec<Reference>>,
    
    /// Documentation
    pub documentation: Option<Documentation>,
    
    /// Related tests
    pub related_tests: Option<Vec<TestInfo>>,
    
    /// Statistics
    pub stats: ContextStats,
}

#[derive(Serialize)]
pub struct SymbolDefinition {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub body: Option<String>,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub visibility: String,
}

#[derive(Serialize)]
pub struct Caller {
    pub file: String,
    pub line: u32,
    pub function: String,
    pub snippet: String,
}

#[derive(Serialize)]
pub struct Callee {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
}

#[derive(Serialize)]
pub struct Reference {
    pub file: String,
    pub line: u32,
    pub context: String,
}

#[derive(Serialize)]
pub struct Documentation {
    pub doc_comment: String,
    pub inline_comments: Vec<String>,
    pub related_docs: Vec<String>,  // README mentions, etc.
}

#[derive(Serialize)]
pub struct TestInfo {
    pub test_name: String,
    pub file: String,
    pub line: u32,
    pub test_type: String,  // unit, integration, e2e
}

#[derive(Serialize)]
pub struct ContextStats {
    pub total_callers: usize,
    pub total_callees: usize,
    pub total_references: usize,
    pub total_tests: usize,
    pub query_time_ms: u64,
}
```

---

## 💻 Implementation Details

### Main Handler

```rust
// src/daemon/tools/get_symbol_context.rs

pub async fn handle_get_symbol_context(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
    lance: &LanceStorage,
) -> Result<Value> {
    let symbol_name = args.get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("symbol required"))?;
    
    let include_callers = args.get("include_callers")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let include_callees = args.get("include_callees")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let include_references = args.get("include_references")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let include_tests = args.get("include_tests")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let max_callers = args.get("max_callers")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    
    let start = Instant::now();
    
    // 1. Find symbol definition
    let definition = find_symbol_definition(sqlite, symbol_name).await?;
    
    // 2. Parallel fetch все связанные данные
    let (callers, callees, references, documentation, tests) = tokio::join!(
        async {
            if include_callers {
                Some(get_callers_internal(sqlite, symbol_name, max_callers).await.ok()?)
            } else {
                None
            }
        },
        async {
            if include_callees {
                Some(get_callees_internal(sqlite, symbol_name).await.ok()?)
            } else {
                None
            }
        },
        async {
            if include_references {
                Some(get_references_internal(sqlite, symbol_name).await.ok()?)
            } else {
                None
            }
        },
        async {
            Some(extract_documentation(sqlite, &definition).await.ok()?)
        },
        async {
            if include_tests {
                Some(find_related_tests(sqlite, symbol_name).await.ok()?)
            } else {
                None
            }
        }
    );
    
    let query_time = start.elapsed().as_millis() as u64;
    
    // 3. Build stats
    let stats = ContextStats {
        total_callers: callers.as_ref().map(|c| c.len()).unwrap_or(0),
        total_callees: callees.as_ref().map(|c| c.len()).unwrap_or(0),
        total_references: references.as_ref().map(|r| r.len()).unwrap_or(0),
        total_tests: tests.as_ref().map(|t| t.len()).unwrap_or(0),
        query_time_ms: query_time,
    };
    
    // 4. Assemble response
    let context = SymbolContext {
        definition,
        callers,
        callees,
        references,
        documentation,
        related_tests: tests,
        stats,
    };
    
    Ok(serde_json::to_value(context)?)
}

async fn find_symbol_definition(
    sqlite: &SqliteStorage,
    symbol_name: &str
) -> Result<SymbolDefinition> {
    let symbol = sqlx::query_as!(
        Symbol,
        "SELECT * FROM symbols WHERE name = ? LIMIT 1",
        symbol_name
    )
    .fetch_one(&sqlite.pool)
    .await?;
    
    // Extract body if needed
    let body = extract_symbol_body(sqlite, &symbol).await?;
    
    Ok(SymbolDefinition {
        name: symbol.name,
        kind: symbol.kind,
        signature: symbol.signature,
        body: Some(body),
        file: symbol.file,
        line_start: symbol.line_start,
        line_end: symbol.line_end,
        visibility: symbol.visibility.unwrap_or_else(|| "private".to_string()),
    })
}

async fn get_callers_internal(
    sqlite: &SqliteStorage,
    symbol_name: &str,
    max: usize
) -> Result<Vec<Caller>> {
    // Reuse existing get_callers logic
    let callers = sqlx::query!(
        r#"
        SELECT file, line, caller_function, snippet
        FROM callers
        WHERE callee = ?
        LIMIT ?
        "#,
        symbol_name,
        max as i64
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    Ok(callers.into_iter().map(|row| Caller {
        file: row.file,
        line: row.line as u32,
        function: row.caller_function,
        snippet: row.snippet,
    }).collect())
}

async fn get_callees_internal(
    sqlite: &SqliteStorage,
    symbol_name: &str
) -> Result<Vec<Callee>> {
    // Reuse existing get_callees logic
    let callees = sqlx::query!(
        r#"
        SELECT name, kind, file, line
        FROM symbols
        WHERE id IN (
            SELECT callee_id FROM calls WHERE caller = ?
        )
        "#,
        symbol_name
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    Ok(callees.into_iter().map(|row| Callee {
        name: row.name,
        kind: row.kind,
        file: row.file,
        line: row.line as u32,
    }).collect())
}

async fn get_references_internal(
    sqlite: &SqliteStorage,
    symbol_name: &str
) -> Result<Vec<Reference>> {
    // Query references table
    let refs = sqlx::query!(
        r#"
        SELECT file, line, context
        FROM references
        WHERE symbol = ?
        "#,
        symbol_name
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    Ok(refs.into_iter().map(|row| Reference {
        file: row.file,
        line: row.line as u32,
        context: row.context,
    }).collect())
}

async fn extract_documentation(
    sqlite: &SqliteStorage,
    definition: &SymbolDefinition
) -> Result<Documentation> {
    // Read file and extract doc comment
    let file_content = sqlite.read_file(&definition.file).await?;
    let lines: Vec<&str> = file_content.lines().collect();
    
    let mut doc_lines = Vec::new();
    
    // Look backwards from symbol line for doc comments
    for i in (0..definition.line_start as usize).rev() {
        let line = lines[i].trim();
        if line.starts_with("///") || line.starts_with("/**") {
            doc_lines.insert(0, line.to_string());
        } else if !line.is_empty() {
            break;
        }
    }
    
    Ok(Documentation {
        doc_comment: doc_lines.join("\n"),
        inline_comments: vec![],
        related_docs: vec![],
    })
}

async fn find_related_tests(
    sqlite: &SqliteStorage,
    symbol_name: &str
) -> Result<Vec<TestInfo>> {
    // Find tests that reference this symbol
    let tests = sqlx::query!(
        r#"
        SELECT name, file, line, kind
        FROM symbols
        WHERE kind = 'test'
          AND (
            name LIKE ? OR
            name LIKE ? OR
            file IN (
              SELECT DISTINCT file FROM references WHERE symbol = ?
            )
          )
        "#,
        format!("%test%{}%", symbol_name),
        format!("%{}%test%", symbol_name),
        symbol_name
    )
    .fetch_all(&sqlite.pool)
    .await?;
    
    Ok(tests.into_iter().map(|row| TestInfo {
        test_name: row.name,
        file: row.file,
        line: row.line as u32,
        test_type: "unit".to_string(),
    }).collect())
}
```

---

## 📈 Success Metrics

### Performance
- ⚡ 2-3× faster than separate calls
- ⏱️ Response time: < 500ms для типичного символа
- 📊 Token savings: 60-70%

### Coverage
- ✅ Finds 100% symbol definitions
- ✅ 95%+ callers/callees accuracy
- ✅ 90%+ related tests found

---

## 📚 Usage Example

```typescript
// Before (6 separate calls)
const location = await gofer.search({query: "authenticate"});
const file = await gofer.read_file({file: location.file});
const callers = await gofer.get_callers({symbol: "authenticate"});
const callees = await gofer.get_callees({symbol: "authenticate"});
const refs = await gofer.get_references({symbol: "authenticate"});
const tests = await gofer.search({query: "test.*authenticate"});

// After (1 unified call)
const context = await gofer.get_symbol_context({
  symbol: "authenticate",
  include_callers: true,
  include_callees: true,
  include_tests: true
});

console.log("Definition:", context.definition);
console.log("Callers:", context.callers.length);
console.log("Callees:", context.callees.length);
console.log("Tests:", context.related_tests.length);
console.log("Query time:", context.stats.query_time_ms, "ms");
```

---

## ✅ Acceptance Criteria

- [ ] Single tool replaces 6 separate calls
- [ ] Parallel execution of sub-queries
- [ ] 60-70% token savings
- [ ] 2-3× faster response time
- [ ] All data accurate (definition, callers, callees, etc.)
- [ ] Response time < 500ms
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** КРИТИЧЕСКИЙ - это самая важная оптимизация. Используется в 80%+ workflows.
