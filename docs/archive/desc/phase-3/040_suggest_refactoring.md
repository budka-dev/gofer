# Feature: suggest_refactoring - Refactoring Recommendations

**ID:** PHASE3-040  
**Priority:** 🔥 Low  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 3 (Intelligence & Security)

---

## 📋 Описание

AI-powered рекомендации по рефакторингу: extract method, simplify conditions, reduce nesting, improve naming.

### Проблема

```
Complex nested conditions
→ Hard to read
→ Potential for bugs
```

### Решение

```typescript
const suggestions = await gofer.suggest_refactoring({
  file: "payment.rs"
});

// Returns:
// Line 45: Extract method "validate_payment"
// Line 89: Simplify nested if/else
// Line 120: Rename variable 'x' to 'userId'
```

---

## 🎯 Goals

- ✅ Multiple refactoring types
- ✅ Context-aware suggestions
- ✅ Priority ranking

---

## ✅ Acceptance Criteria

- [ ] Suggestions actionable
- [ ] Priority ranking makes sense
- [ ] Multiple refactoring types

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
