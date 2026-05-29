# Feature: analyze_query_performance - Query Optimization Analysis

**ID:** PHASE2-030  
**Priority:** 🔥🔥 Medium  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 2 (Database Intelligence)

---

## 📋 Описание

Анализ SQL queries: EXPLAIN plan, missing indexes, optimization recommendations. Помогает находить slow queries и предлагает improvements.

### Проблема

```
AI: "Почему этот query медленный?"
→ Нет EXPLAIN analysis

Developer: "Какие indexes нужны?"
→ Нет automated recommendations
```

### Решение

```typescript
const analysis = await gofer.analyze_query_performance({
  query: "SELECT * FROM users WHERE email = '...'"
});

// Returns:
// Execution plan: Sequential Scan (SLOW)
// Missing index: email column
// Recommendation: CREATE INDEX idx_users_email ON users(email)
// Estimated speedup: 100×
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ EXPLAIN plan analysis
- ✅ Detect missing indexes
- ✅ Optimization recommendations
- ✅ Estimated performance improvement

### Non-Goals
- ❌ Не automatic index creation
- ❌ Не query rewriting

---

## 🔧 API Specification

```json
{
  "name": "analyze_query_performance",
  "description": "Анализ производительности SQL query",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {"type": "string"},
      "connection": {"type": "string"}
    },
    "required": ["query"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct QueryAnalysis {
    pub query: String,
    pub execution_plan: ExecutionPlan,
    pub missing_indexes: Vec<IndexRecommendation>,
    pub optimization_opportunities: Vec<Optimization>,
    pub estimated_cost: f32,
}

#[derive(Serialize)]
pub struct ExecutionPlan {
    pub plan_type: String,
    pub cost: f32,
    pub rows: usize,
    pub details: String,
}

#[derive(Serialize)]
pub struct IndexRecommendation {
    pub table: String,
    pub columns: Vec<String>,
    pub index_type: String,
    pub estimated_speedup: f32,
    pub sql: String,
}

#[derive(Serialize)]
pub struct Optimization {
    pub issue: String,
    pub recommendation: String,
    pub impact: ImpactLevel,
}
```

---

## 💻 Implementation

```rust
pub async fn analyze_query_performance(
    query: &str,
    connection: &str
) -> Result<QueryAnalysis> {
    let pool = PgPool::connect(connection).await?;
    
    // 1. Get EXPLAIN plan
    let explain_query = format!("EXPLAIN (FORMAT JSON) {}", query);
    let plan: serde_json::Value = sqlx::query_scalar(&explain_query)
        .fetch_one(&pool)
        .await?;
    
    let execution_plan = parse_explain_plan(&plan)?;
    
    // 2. Detect missing indexes
    let missing_indexes = detect_missing_indexes(&plan, query)?;
    
    // 3. Find optimization opportunities
    let optimizations = find_optimizations(&plan, query)?;
    
    Ok(QueryAnalysis {
        query: query.to_string(),
        execution_plan,
        missing_indexes,
        optimization_opportunities: optimizations,
        estimated_cost: plan["Plan"]["Total Cost"].as_f64().unwrap() as f32,
    })
}

fn detect_missing_indexes(
    plan: &serde_json::Value,
    query: &str
) -> Result<Vec<IndexRecommendation>> {
    let mut recommendations = Vec::new();
    
    // Look for Sequential Scans
    if plan["Plan"]["Node Type"].as_str() == Some("Seq Scan") {
        let table = plan["Plan"]["Relation Name"].as_str().unwrap();
        
        // Parse WHERE conditions
        let where_columns = extract_where_columns(query)?;
        
        for column in where_columns {
            recommendations.push(IndexRecommendation {
                table: table.to_string(),
                columns: vec![column.clone()],
                index_type: "BTREE".to_string(),
                estimated_speedup: 100.0, // Heuristic
                sql: format!("CREATE INDEX idx_{}_{} ON {} ({})", 
                    table, column, table, column),
            });
        }
    }
    
    Ok(recommendations)
}
```

---

## 📈 Success Metrics

- ✅ EXPLAIN plan parsed correctly
- ✅ 80%+ relevant index recommendations
- ⏱️ Response time < 3s

---

## ✅ Acceptance Criteria

- [ ] EXPLAIN plan analysis works
- [ ] Detects missing indexes
- [ ] Recommendations actionable
- [ ] Estimated speedup reasonable
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
