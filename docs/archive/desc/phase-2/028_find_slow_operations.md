# Feature: find_slow_operations - Performance Bottlenecks

**ID:** PHASE2-028  
**Priority:** 🔥🔥 Medium  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 2 (Production Observability)

---

## 📋 Описание

Поиск самых медленных операций в production: endpoints, database queries, external API calls. Ранжирование по impact (frequency × latency).

### Проблема

```
AI: "Что тормозит систему?"
→ Нет overview медленных операций

Developer: "Где оптимизировать первым делом?"
→ Не знаем impact каждой операции
```

### Решение

```typescript
const slow = await gofer.find_slow_operations({
  limit: 10
});

// Returns:
// 1. GET /api/users - p95: 1.2s, 1000 req/s → Impact: HIGH
// 2. SELECT * FROM orders - 450ms, 500 q/s → Impact: MEDIUM
// 3. External API: stripe.com - 800ms, 100 req/s → Impact: LOW
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Find slowest endpoints/queries
- ✅ Rank by impact (freq × latency)
- ✅ Categorize by type (DB, API, function)
- ✅ Actionable recommendations

### Non-Goals
- ❌ Не automatic optimization
- ❌ Не profiling (use APM tools)

---

## 🔧 API Specification

```json
{
  "name": "find_slow_operations",
  "description": "Найти медленные операции в production",
  "inputSchema": {
    "type": "object",
    "properties": {
      "limit": {"type": "number", "default": 10},
      "min_latency_ms": {"type": "number", "default": 100}
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct SlowOperation {
    pub operation_type: OperationType,
    pub name: String,
    pub latency_p95: f32,
    pub frequency: f32,
    pub impact_score: f32,
    pub recommendation: String,
}

#[derive(Serialize)]
pub enum OperationType {
    Endpoint,
    DatabaseQuery,
    ExternalAPI,
    Function,
}
```

---

## 💻 Implementation

```rust
pub async fn find_slow_operations(limit: usize) -> Result<Vec<SlowOperation>> {
    let prom = PrometheusClient::new()?;
    
    // Query slow endpoints
    let query = r#"
        topk(10,
          (histogram_quantile(0.95, rate(http_request_duration_seconds_bucket[5m])))
          *
          (rate(http_request_duration_seconds_count[5m]))
        )
    "#;
    
    let results = prom.query(query).await?;
    
    let mut operations = Vec::new();
    
    for result in results {
        let impact = result.latency * result.frequency;
        
        operations.push(SlowOperation {
            operation_type: OperationType::Endpoint,
            name: result.name,
            latency_p95: result.latency,
            frequency: result.frequency,
            impact_score: impact,
            recommendation: generate_recommendation(&result),
        });
    }
    
    // Sort by impact
    operations.sort_by(|a, b| 
        b.impact_score.partial_cmp(&a.impact_score).unwrap()
    );
    
    operations.truncate(limit);
    
    Ok(operations)
}

fn generate_recommendation(op: &SlowOperation) -> String {
    match op.operation_type {
        OperationType::DatabaseQuery => {
            "Consider adding index or query optimization".to_string()
        }
        OperationType::ExternalAPI => {
            "Consider caching or async processing".to_string()
        }
        _ => "Profile and optimize hot path".to_string()
    }
}
```

---

## 📈 Success Metrics

- ✅ Identifies real bottlenecks
- ✅ Impact ranking accurate
- ⏱️ Response time < 2s

---

## ✅ Acceptance Criteria

- [ ] Finds slow operations
- [ ] Ranks by impact
- [ ] Recommendations helpful
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
