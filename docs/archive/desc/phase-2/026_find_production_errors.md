# Feature: find_production_errors - Production Error Analysis

**ID:** PHASE2-026  
**Priority:** 🔥🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 2 (Production Observability)

---

## 📋 Описание

Анализ production errors связанных с конкретным кодом. Показывает frequency, affected users, first/last seen timestamps.

### Проблема

```
AI: "Есть ли ошибки в этом коде в production?"
→ Нет visibility в production errors

Developer: "Как часто это падает?"
→ Error frequency не известна
```

### Решение

```typescript
const errors = await gofer.find_production_errors({
  file: "src/payment.rs",
  time_range: "7d"
});

// Returns:
// payment.rs:234 - PaymentTimeout
//   Frequency: 45 errors/hour (CRITICAL)
//   Affected users: 1,234
//   First seen: 2026-02-14 10:23
//   Last seen: 5 minutes ago
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Filter logs by file/function
- ✅ Error frequency metrics
- ✅ User impact analysis
- ✅ Temporal patterns

### Non-Goals
- ❌ Не automatic error fixing
- ❌ Не alerting (use existing tools)

---

## 🔧 API Specification

```json
{
  "name": "find_production_errors",
  "description": "Найти production ошибки для кода",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "function": {"type": "string", "description": "Optional"},
      "time_range": {"type": "string", "default": "24h"}
    },
    "required": ["file"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct ProductionError {
    pub error_type: String,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub frequency: ErrorFrequency,
    pub affected_users: usize,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub severity: ErrorSeverity,
}

#[derive(Serialize)]
pub struct ErrorFrequency {
    pub count: usize,
    pub rate: f32,  // errors per hour
}

#[derive(Serialize)]
pub enum ErrorSeverity {
    Low,      // < 1 error/hour
    Medium,   // 1-10 errors/hour
    High,     // 10-100 errors/hour
    Critical, // > 100 errors/hour
}
```

---

## 💻 Implementation

```rust
pub async fn find_production_errors(
    file: &str,
    time_range: &str
) -> Result<Vec<ProductionError>> {
    // Query logs filtered by file
    let logs = search_logs(&format!("file:{}", file), time_range, "error").await?;
    
    // Group by error type + location
    let mut error_groups: HashMap<String, Vec<LogEntry>> = HashMap::new();
    
    for log in logs {
        if let Some(ref loc) = log.code_location {
            if loc.file == file {
                let key = format!("{}:{}:{}", log.message, loc.file, loc.line);
                error_groups.entry(key).or_default().push(log);
            }
        }
    }
    
    // Analyze each group
    let mut errors = Vec::new();
    
    for (_, group) in error_groups {
        let first = group.first().unwrap();
        let last = group.last().unwrap();
        
        let frequency = ErrorFrequency {
            count: group.len(),
            rate: calculate_rate(group.len(), time_range),
        };
        
        let severity = match frequency.rate {
            r if r < 1.0 => ErrorSeverity::Low,
            r if r < 10.0 => ErrorSeverity::Medium,
            r if r < 100.0 => ErrorSeverity::High,
            _ => ErrorSeverity::Critical,
        };
        
        errors.push(ProductionError {
            error_type: first.message.clone(),
            message: first.message.clone(),
            file: first.code_location.as_ref().unwrap().file.clone(),
            line: first.code_location.as_ref().unwrap().line,
            frequency,
            affected_users: count_affected_users(&group),
            first_seen: first.timestamp,
            last_seen: last.timestamp,
            severity,
        });
    }
    
    Ok(errors)
}

fn calculate_rate(count: usize, time_range: &str) -> f32 {
    let hours = parse_time_range_hours(time_range);
    count as f32 / hours
}

fn count_affected_users(logs: &[LogEntry]) -> usize {
    logs.iter()
        .filter_map(|log| log.context.get("user_id"))
        .collect::<HashSet<_>>()
        .len()
}
```

---

## 📈 Success Metrics

- ✅ Accurate error frequency
- ✅ User impact correct
- ⏱️ Response time < 3s

---

## ✅ Acceptance Criteria

- [ ] Filters by file/function
- [ ] Calculates frequency
- [ ] Counts affected users
- [ ] Severity assessment accurate
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
