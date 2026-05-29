# Feature: get_code_churn - Метрики изменчивости кода

**ID:** PHASE1-015  
**Priority:** 🔥🔥 Medium  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context)

---

## 📋 Описание

Анализ частоты изменений файлов для выявления нестабильных областей кода. Высокий churn часто указывает на проблемы в дизайне.

### Проблема

```
Question: "Какие файлы нестабильны?"
→ Без churn metrics непонятно где технические долги

Question: "Где вероятны регрессии?"
→ Frequent changes = higher bug risk
```

### Решение

```typescript
const churn = await gofer.get_code_churn({
  period: "3 months",
  threshold: 10
});

// Returns:
// src/server.rs: 47 изменений (CRITICAL)
// src/auth.rs: 23 изменения (HIGH)
// Recommendation: "Consider refactoring server.rs"
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Churn metrics по файлам
- ✅ Temporal analysis (по периодам)
- ✅ Recommendations

### Non-Goals
- ❌ Не automatic refactoring

---

## 🔧 API Specification

```json
{
  "name": "get_code_churn",
  "description": "Анализ частоты изменений файлов",
  "inputSchema": {
    "type": "object",
    "properties": {
      "period": {"type": "string", "default": "3 months"},
      "threshold": {"type": "number", "default": 5}
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct ChurnMetrics {
    pub file: String,
    pub commit_count: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub churn_score: f32,
    pub risk_level: RiskLevel,
}
```

---

## 💻 Implementation

```rust
pub async fn get_code_churn(period: &str) -> Result<Vec<ChurnMetrics>> {
    let output = Command::new("git")
        .args(&["log", "--since", period, "--numstat", "--format="])
        .output()?;
    
    let metrics = aggregate_changes_per_file(&output.stdout)?;
    
    Ok(metrics)
}
```

---

## 📈 Success Metrics

- ✅ Accurate churn calculation
- ⏱️ Response time < 3s

---

## ✅ Acceptance Criteria

- [ ] Calculates churn per file
- [ ] Risk levels accurate
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
