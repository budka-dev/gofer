# Feature: detect_code_smells - Code Quality Analysis

**ID:** PHASE3-038  
**Priority:** 🔥 Low  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 3 (Intelligence & Security)

---

## 📋 Описание

Обнаружение code smells: long functions, duplicate code, god classes, deep nesting.

### Проблема

```
Function: 500 lines (too long)
God class: 50 methods (violates SRP)
Duplicate code: 80% similarity
```

### Решение

```typescript
const smells = await gofer.detect_code_smells();

// Returns:
// UserManager.rs - God Class (53 methods)
// process() - Long Function (423 lines)
// auth.rs & auth2.rs - Duplicate Code (85%)
```

---

## 🎯 Goals

- ✅ Detect: long functions, god classes, duplication
- ✅ Severity ranking
- ✅ Refactoring suggestions

---

## ✅ Acceptance Criteria

- [ ] Multiple smell types detected
- [ ] Accurate detection
- [ ] Actionable recommendations

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
