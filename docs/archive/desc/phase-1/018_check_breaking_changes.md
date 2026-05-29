# Feature: check_breaking_changes - Детекция breaking changes

**ID:** PHASE1-018  
**Priority:** 🔥🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context - Real-time Change Impact)

---

## 📋 Описание

Автоматическое обнаружение breaking changes в public API. Проверяет изменения сигнатур exported функций и предупреждает о potential breakage.

### Проблема

```
Developer: изменил public function signature
Question: "Это breaking change?"
→ Без анализа можно сломать внешние модули

Developer: удалил public struct field
→ Незаметно broke API compatibility
```

### Решение

```typescript
const breaking = await gofer.check_breaking_changes();

// Returns:
// BREAKING: authenticate(token) → authenticate(token, options)
//   - Signature changed
//   - 5 external callers affected
//   - Recommendation: add default parameter value
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Detect public API changes
- ✅ Compare signatures (before/after)
- ✅ Find external callers
- ✅ Severity assessment

### Non-Goals
- ❌ Не automatic migration
- ❌ Не semantic versioning (только detection)

---

## 🔧 API Specification

```json
{
  "name": "check_breaking_changes",
  "description": "Обнаружить breaking changes в public API",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct BreakingChange {
    pub symbol: String,
    pub kind: BreakingChangeKind,
    pub old_signature: String,
    pub new_signature: String,
    pub affected_callers: Vec<CallerLocation>,
    pub severity: Severity,
    pub recommendation: String,
}

#[derive(Serialize)]
pub enum BreakingChangeKind {
    SignatureChanged,
    Removed,
    VisibilityReduced,
    TypeChanged,
}

#[derive(Serialize)]
pub enum Severity {
    Minor,    // < 5 callers
    Major,    // 5-20 callers
    Critical, // > 20 callers
}
```

---

## 💻 Implementation

```rust
pub async fn check_breaking_changes() -> Result<Vec<BreakingChange>> {
    let impact = analyze_uncommitted_changes().await?;
    
    let mut breaking = Vec::new();
    
    for symbol in impact.modified_symbols {
        // Only check public/exported symbols
        if !matches!(symbol.visibility, Visibility::Public) {
            continue;
        }
        
        let kind = match symbol.change_type {
            ChangeType::SignatureChanged => BreakingChangeKind::SignatureChanged,
            ChangeType::Removed => BreakingChangeKind::Removed,
            _ => continue,
        };
        
        // Find affected callers
        let callers = find_external_callers(&symbol.name).await?;
        
        let severity = match callers.len() {
            0..=4 => Severity::Minor,
            5..=20 => Severity::Major,
            _ => Severity::Critical,
        };
        
        breaking.push(BreakingChange {
            symbol: symbol.name,
            kind,
            old_signature: symbol.old_signature.unwrap_or_default(),
            new_signature: symbol.new_signature.unwrap_or_default(),
            affected_callers: callers,
            severity,
            recommendation: generate_recommendation(&kind, callers.len()),
        });
    }
    
    Ok(breaking)
}
```

---

## 📈 Success Metrics

- ✅ Detects 100% breaking changes
- ✅ No false positives on private changes
- ⏱️ Response time < 2s

---

## ✅ Acceptance Criteria

- [ ] Detects signature changes
- [ ] Detects removals
- [ ] Only checks public API
- [ ] Severity assessment accurate
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
