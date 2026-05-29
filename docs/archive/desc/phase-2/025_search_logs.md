# Feature: search_logs - Production Log Search

**ID:** PHASE2-025  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 2 (Production Observability)

---

## 📋 Описание

Интеграция с Elasticsearch/Loki для поиска production logs. Маппинг stack traces на code locations.

### Проблема

```
Production error logged:
"NullPointerException at auth.rs:145"
→ Logs в Elasticsearch, но gofer не знает об этом

AI: "Какие ошибки в production?"
→ Нет доступа к production observability
```

### Решение

```typescript
const logs = await gofer.search_logs({
  query: "authentication error",
  time_range: "24h"
});

// Returns:
// 47 matching entries
// auth.rs:145 - NullPointerException (12 occurrences)
// auth.rs:230 - Timeout (8 occurrences)
// Stack traces mapped to code
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Elasticsearch/Loki integration
- ✅ Parse stack traces
- ✅ Map errors → code locations
- ✅ Frequency analysis

### Non-Goals
- ❌ Не log aggregation (use existing tools)
- ❌ Не alerting

---

## 🔧 API Specification

```json
{
  "name": "search_logs",
  "description": "Поиск production logs",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {"type": "string"},
      "time_range": {"type": "string", "default": "24h"},
      "severity": {
        "type": "string",
        "enum": ["error", "warn", "info"],
        "default": "error"
      },
      "limit": {"type": "number", "default": 100}
    },
    "required": ["query"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub message: String,
    pub stack_trace: Option<StackTrace>,
    pub code_location: Option<CodeLocation>,
    pub context: HashMap<String, String>,
}

#[derive(Serialize)]
pub struct StackTrace {
    pub frames: Vec<StackFrame>,
}

#[derive(Serialize)]
pub struct StackFrame {
    pub file: String,
    pub line: u32,
    pub function: String,
}

#[derive(Serialize)]
pub struct CodeLocation {
    pub file: String,
    pub line: u32,
}
```

---

## 💻 Implementation

```rust
pub async fn search_logs(
    query: &str,
    time_range: &str,
    severity: &str
) -> Result<Vec<LogEntry>> {
    // Elasticsearch query
    let es_query = json!({
        "query": {
            "bool": {
                "must": [
                    {"match": {"message": query}},
                    {"range": {"@timestamp": {"gte": format!("now-{}", time_range)}}}
                ],
                "filter": [
                    {"term": {"level": severity}}
                ]
            }
        }
    });
    
    let response = es_client.search(&es_query).await?;
    
    let mut entries = Vec::new();
    
    for hit in response.hits {
        let mut entry: LogEntry = serde_json::from_value(hit.source)?;
        
        // Parse stack trace
        if let Some(ref st) = entry.stack_trace {
            entry.code_location = extract_code_location(st);
        }
        
        entries.push(entry);
    }
    
    Ok(entries)
}

fn extract_code_location(stack_trace: &StackTrace) -> Option<CodeLocation> {
    // Find first frame in our codebase
    stack_trace.frames.iter()
        .find(|f| f.file.starts_with("src/"))
        .map(|f| CodeLocation {
            file: f.file.clone(),
            line: f.line,
        })
}
```

---

## 📈 Success Metrics

- ✅ Finds relevant logs
- ✅ Stack traces parsed correctly
- ⏱️ Response time < 5s

---

## ✅ Acceptance Criteria

- [ ] Elasticsearch integration
- [ ] Stack trace parsing
- [ ] Code location mapping
- [ ] Time range filtering
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
