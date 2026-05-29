# Feature: find_all_todos - Поиск TODO комментариев

**ID:** PHASE1-014  
**Priority:** 🔥 Low  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context)

---

## 📋 Описание

Поиск и агрегация всех TODO/FIXME/HACK комментариев в проекте с группировкой и приоритизацией.

### Проблема

```
AI: "Что нужно доделать?"
→ TODO разбросаны по файлам, нет overview

AI: "Какие известные проблемы?"
→ FIXME/HACK не видны в одном месте
```

### Решение

```typescript
const todos = await gofer.find_all_todos();

// Returns:
// 47 TODO items
// 12 FIXME items
// 5 HACK items
// Grouped by module, prioritized
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Find all TODO/FIXME/HACK/XXX
- ✅ Group by module
- ✅ Prioritize by importance

### Non-Goals
- ❌ Не automatic fixing
- ❌ Не tracking completion

---

## 🔧 API Specification

```json
{
  "name": "find_all_todos",
  "description": "Найти все TODO/FIXME/HACK комментарии",
  "inputSchema": {
    "type": "object",
    "properties": {
      "types": {
        "type": "array",
        "items": {"type": "string"},
        "default": ["TODO", "FIXME", "HACK", "XXX"]
      }
    }
  }
}
```

---

## 💻 Implementation

```rust
pub async fn find_all_todos() -> Result<Vec<TodoItem>> {
    // Grep pattern: TODO|FIXME|HACK|XXX
    let output = Command::new("rg")
        .args(&["-n", r"(TODO|FIXME|HACK|XXX):", "."])
        .output()?;
    
    let items = parse_todo_items(&output.stdout)?;
    
    // Group by module
    let grouped = group_by_module(items);
    
    Ok(grouped)
}
```

---

## 📈 Success Metrics

- ✅ Finds 100% TODO comments
- ⏱️ Response time < 2s for 1000 files

---

## ✅ Acceptance Criteria

- [ ] Finds all TODO/FIXME/HACK
- [ ] Groups by module
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
