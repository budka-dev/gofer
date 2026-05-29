# Feature: get_design_decisions - Архитектурные решения

**ID:** PHASE2-022  
**Priority:** 🔥🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 2 (Human Context)

---

## 📋 Описание

Извлечение архитектурных решений (ADR - Architecture Decision Records) и design rationale из документации и commit messages. Отвечает на вопрос "Почему так спроектировано?"

### Проблема

```
AI: "Почему используется event sourcing?"
→ Без ADR не понятно почему было принято это решение

Developer: "Какие были альтернативы?"
→ Design rationale не документирован в коде
```

### Решение

```typescript
const decisions = await gofer.get_design_decisions({
  module: "auth"
});

// Returns:
// ADR-001: "Use JWT for authentication"
//   Decision: JWT instead of session cookies
//   Rationale: Stateless, scalable
//   Alternatives: Sessions (rejected: scaling issues)
//   Date: 2025-03-15
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Parse ADR files (docs/adr/*.md)
- ✅ Extract design rationale from commits
- ✅ Link decisions to code modules
- ✅ Show alternatives considered

### Non-Goals
- ❌ Не создает ADR автоматически
- ❌ Не validates decisions

---

## 🔧 API Specification

```json
{
  "name": "get_design_decisions",
  "description": "Получить архитектурные решения для модуля",
  "inputSchema": {
    "type": "object",
    "properties": {
      "module": {"type": "string"}
    },
    "required": ["module"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct ArchitectureDecision {
    pub id: String,
    pub title: String,
    pub decision: String,
    pub rationale: String,
    pub alternatives: Vec<Alternative>,
    pub status: DecisionStatus,
    pub date: DateTime<Utc>,
    pub related_files: Vec<String>,
}

#[derive(Serialize)]
pub struct Alternative {
    pub name: String,
    pub rejected_reason: String,
}

#[derive(Serialize)]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Deprecated,
    Superseded,
}
```

---

## 💻 Implementation

```rust
pub async fn get_design_decisions(module: &str) -> Result<Vec<ArchitectureDecision>> {
    let mut decisions = Vec::new();
    
    // 1. Parse ADR files
    let adr_files = glob("docs/adr/*.md")?;
    
    for file in adr_files {
        let content = fs::read_to_string(&file)?;
        
        if content.contains(module) {
            let decision = parse_adr(&content)?;
            decisions.push(decision);
        }
    }
    
    // 2. Extract from commit messages
    let output = Command::new("git")
        .args(&["log", "--grep", &format!("ADR|design|architecture.*{}", module)])
        .output()?;
    
    // Parse commits for design rationale
    
    Ok(decisions)
}

fn parse_adr(content: &str) -> Result<ArchitectureDecision> {
    // Parse markdown ADR format
    // Example:
    // # ADR-001: Use JWT for authentication
    // ## Status: Accepted
    // ## Context: ...
    // ## Decision: ...
    // ## Consequences: ...
    
    todo!("Parse ADR markdown")
}
```

---

## 📈 Success Metrics

- ✅ Finds 90%+ documented decisions
- ✅ Accurate rationale extraction
- ⏱️ Response time < 1s

---

## ✅ Acceptance Criteria

- [ ] Parses ADR files
- [ ] Extracts from commits
- [ ] Links to modules
- [ ] Shows alternatives
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
