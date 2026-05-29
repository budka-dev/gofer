# Feature: find_sql_injection_risks - SQL Injection Detection

**ID:** PHASE3-035  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 3 (Intelligence & Security)

---

## 📋 Описание

AST-based detection SQL injection рисков: string concatenation в SQL queries, format!() с user input, missing parameterization.

### Проблема

```rust
// VULNERABLE:
let query = format!("SELECT * FROM users WHERE id = {}", user_input);
db.execute(&query)?;

// Safe:
let query = "SELECT * FROM users WHERE id = ?";
db.execute(query, &[user_input])?;
```

### Решение

```typescript
const risks = await gofer.find_sql_injection_risks();

// Returns:
// ⚠️ HIGH: auth.rs:45 - String concatenation in SQL query
// ⚠️ MEDIUM: api.rs:123 - format!() with user input
```

---

## 🎯 Goals

- ✅ AST analysis для SQL construction
- ✅ Detect: concatenation, format!(), interpolation
- ✅ Severity based on user input proximity
- ✅ Fix suggestions

---

## 🔧 API

```json
{
  "name": "find_sql_injection_risks",
  "inputSchema": {"type": "object"}
}
```

---

## ✅ Acceptance Criteria

- [ ] Detects string concatenation
- [ ] Detects format!() misuse
- [ ] < 5% false positives
- [ ] Fix suggestions provided

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
