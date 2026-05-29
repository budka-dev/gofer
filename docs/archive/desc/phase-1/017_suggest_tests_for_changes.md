# Feature: suggest_tests_for_changes - Рекомендации по тестированию

**ID:** PHASE1-017  
**Priority:** 🔥🔥 Medium  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context - Real-time Change Impact)

---

## 📋 Описание

AI-powered рекомендации какие тесты запустить на основе измененного кода. Интеграция с historical test failure data для smart prioritization.

### Проблема

```
Developer: изменил auth.rs
Question: "Какие тесты запустить?"
→ Без анализа придется запускать все (долго)
→ Или угадывать (пропустим важные тесты)
```

### Решение

```typescript
const suggestions = await gofer.suggest_tests_for_changes();

// Returns:
// Priority 1: test_authentication() - directly affected
// Priority 2: test_login_flow() - calls modified function
// Priority 3: test_session_*() - historical failures after auth changes
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Map changed functions → related tests
- ✅ Use historical failure data
- ✅ Priority ranking

### Non-Goals
- ❌ Не запускает тесты (только рекомендации)
- ❌ Не генерирует новые тесты

---

## 🔧 API Specification

```json
{
  "name": "suggest_tests_for_changes",
  "description": "Рекомендовать какие тесты запустить на основе изменений",
  "inputSchema": {
    "type": "object",
    "properties": {
      "limit": {"type": "number", "default": 10}
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct TestSuggestion {
    pub test_name: String,
    pub priority: Priority,
    pub reason: String,
    pub historical_failures: usize,
}

#[derive(Serialize)]
pub enum Priority {
    Critical,  // Directly tests modified code
    High,      // Calls modified functions
    Medium,    // Same module
    Low,       // Historical correlation
}
```

---

## 💻 Implementation

```rust
pub async fn suggest_tests_for_changes() -> Result<Vec<TestSuggestion>> {
    // 1. Get modified symbols
    let impact = analyze_uncommitted_changes().await?;
    
    // 2. Find tests for modified functions
    let mut suggestions = Vec::new();
    
    for symbol in impact.modified_symbols {
        // Find tests that directly test this symbol
        let direct_tests = find_tests_for_symbol(&symbol.name).await?;
        
        for test in direct_tests {
            suggestions.push(TestSuggestion {
                test_name: test,
                priority: Priority::Critical,
                reason: format!("Tests modified function {}", symbol.name),
                historical_failures: 0,
            });
        }
        
        // Find tests that call modified functions
        let affected = impact.affected_callers.iter()
            .filter(|c| c.needs_update)
            .collect::<Vec<_>>();
        
        for caller in affected {
            let indirect_tests = find_tests_for_file(&caller.file).await?;
            // Add to suggestions...
        }
    }
    
    // 3. Add historical failure data
    enhance_with_historical_data(&mut suggestions).await?;
    
    // 4. Sort by priority
    suggestions.sort_by_key(|s| s.priority);
    
    Ok(suggestions)
}
```

---

## 📈 Success Metrics

- ✅ Suggests 90%+ relevant tests
- ✅ Critical tests always in top 5
- ⏱️ Response time < 2s

---

## ✅ Acceptance Criteria

- [ ] Maps changes to tests
- [ ] Priority ranking accurate
- [ ] Historical data integrated
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
