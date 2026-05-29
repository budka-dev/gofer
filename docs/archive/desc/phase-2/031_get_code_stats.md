# Feature: get_code_stats - Code Analytics & Metrics

**ID:** PHASE2-031  
**Priority:** 🔥🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 2 (Analytics & Monitoring)

---

## 📋 Описание

Pre-computed code metrics и analytics: количество функций, test coverage, сложность, размер кодовой базы. Fast queries через pre-aggregated data.

### Проблема

```
AI: "Сколько API endpoints в проекте?"
→ Нужно сканировать весь код (медленно)

Developer: "Какой test coverage?"
→ Нет aggregated metrics
```

### Решение

```typescript
const stats = await gofer.get_code_stats({
  metric: "api_count"
});

// Returns: 247 API endpoints
// Instant response (pre-computed)

const coverage = await gofer.get_code_stats({
  metric: "test_coverage"
});
// Returns: 78.5% coverage
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Pre-computed metrics
- ✅ Fast queries (< 100ms)
- ✅ Background aggregation
- ✅ Multiple metric types

### Non-Goals
- ❌ Не real-time (periodic updates OK)
- ❌ Не detailed breakdown (use specific tools)

---

## 🔧 API Specification

```json
{
  "name": "get_code_stats",
  "description": "Получить агрегированные метрики кодовой базы",
  "inputSchema": {
    "type": "object",
    "properties": {
      "metric": {
        "type": "string",
        "enum": ["api_count", "function_count", "test_coverage", "total_lines", "avg_complexity"]
      }
    },
    "required": ["metric"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct CodeStats {
    pub metric: String,
    pub value: f64,
    pub breakdown: Option<HashMap<String, f64>>,
    pub last_updated: DateTime<Utc>,
}
```

---

## 💻 Implementation

```rust
// Store metrics in SQLite
CREATE TABLE code_metrics (
    metric_name TEXT PRIMARY KEY,
    value REAL NOT NULL,
    breakdown TEXT,  -- JSON
    updated_at DATETIME NOT NULL
);

pub async fn get_code_stats(metric: &str) -> Result<CodeStats> {
    // Query pre-computed metric
    let row = sqlx::query!(
        "SELECT value, breakdown, updated_at FROM code_metrics WHERE metric_name = ?",
        metric
    )
    .fetch_one(&pool)
    .await?;
    
    Ok(CodeStats {
        metric: metric.to_string(),
        value: row.value,
        breakdown: row.breakdown.map(|b| serde_json::from_str(&b).ok()).flatten(),
        last_updated: row.updated_at,
    })
}

// Background job: update metrics
pub async fn update_metrics_job() {
    loop {
        // Update every 1 hour
        tokio::time::sleep(Duration::from_secs(3600)).await;
        
        let api_count = count_api_endpoints().await?;
        let function_count = count_functions().await?;
        let test_coverage = calculate_test_coverage().await?;
        
        store_metric("api_count", api_count as f64).await?;
        store_metric("function_count", function_count as f64).await?;
        store_metric("test_coverage", test_coverage).await?;
    }
}
```

---

## 📈 Success Metrics

- ⚡ Response time < 100ms
- ✅ Metrics accurate (±2%)
- 🔄 Updates every hour

---

## ✅ Acceptance Criteria

- [ ] Pre-computed metrics stored
- [ ] Fast queries (< 100ms)
- [ ] Background aggregation works
- [ ] Multiple metrics supported
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
