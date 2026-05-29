# Feature: find_hotspots - Анализ нестабильных участков

**ID:** PHASE1-013  
**Priority:** 🔥🔥 Medium  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context)

---

## 📋 Описание

Определение "горячих точек" в коде - участков с высокой частотой изменений, которые могут указывать на проблемные области требующие рефакторинга.

### Проблема

```
Question: "Какие части системы нестабильны?"
→ Без churn analysis не понять где накапливаются технические долги

Question: "Где вероятны баги?"
→ Hotspots часто коррелируют с bugs
```

### Решение

```typescript
const hotspots = await gofer.find_hotspots({
  file: "src/server.rs"
});

// Returns:
// Lines 120-145: 23 изменения за 3 месяца (HIGH risk)
// Lines 78-92: 12 изменений (MEDIUM risk)
// Recommendation: "Consider refactoring handle_request()"
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Churn analysis по строкам
- ✅ Корреляция с багами (если есть issue links)
- ✅ Рекомендации по рефакторингу

### Non-Goals
- ❌ Не automatic refactoring

---

## 🔧 API Specification

```json
{
  "name": "find_hotspots",
  "description": "Найти нестабильные участки кода с частыми изменениями",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "threshold": {"type": "number", "default": 5, "description": "Min changes to consider hotspot"}
    },
    "required": ["file"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct Hotspot {
    pub line_start: u32,
    pub line_end: u32,
    pub change_count: usize,
    pub risk_level: RiskLevel,
    pub context: String,  // function/class name
    pub recommendation: Option<String>,
}

#[derive(Serialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}
```

---

## 💻 Implementation

```rust
pub async fn find_hotspots(file: &str) -> Result<Vec<Hotspot>> {
    // git log -L для line-level history
    let output = Command::new("git")
        .args(&["log", "-L", &format!(":{}:", file)])
        .output()?;
    
    // Aggregate changes per line range
    let line_changes = aggregate_line_changes(&output.stdout)?;
    
    // Find hotspots (threshold: 5+ changes)
    let hotspots = line_changes.into_iter()
        .filter(|(_, count)| *count >= 5)
        .map(|(range, count)| {
            let risk = match count {
                0..=5 => RiskLevel::Low,
                6..=10 => RiskLevel::Medium,
                11..=20 => RiskLevel::High,
                _ => RiskLevel::Critical,
            };
            
            Hotspot {
                line_start: range.start,
                line_end: range.end,
                change_count: count,
                risk_level: risk,
                context: find_context(file, range.start)?,
                recommendation: generate_recommendation(risk, count),
            }
        })
        .collect();
    
    Ok(hotspots)
}
```

---

## 📈 Success Metrics

- ✅ Identifies 90%+ actual problem areas
- ⏱️ Response time < 5s
- 💡 Actionable recommendations

---

## ✅ Acceptance Criteria

- [ ] Detects hotspots accurately
- [ ] Risk levels make sense
- [ ] Recommendations are helpful
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
