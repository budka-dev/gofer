# Feature: read_types_only - Чтение только типов

**ID:** PHASE0-010  
**Priority:** 🔥🔥 Medium  
**Effort:** 1 день  
**Status:** Not Started  
**Phase:** 0 (Token-Efficient Reading)

---

## 📋 Описание

MCP tool для извлечения только определений типов из файла (structs, enums, interfaces, type aliases) без функций и implementation blocks. Экономит токены при анализе data models и API contracts.

### Проблема

```
AI: "Какие типы данных используются для User?"

Without read_types_only:
- read_file("models/user.rs") - 1500 строк
- Structs: 200 строк ✅
- Functions: 1000 строк ❌
- Tests: 300 строк ❌
Total: 1500 строк, ~3500 токенов (только 13% полезно)

With read_types_only:
- Types only: 72 строки, ~150 токенов
- Экономия: 96% токенов!
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Извлечь все type definitions
- ✅ Поддержка: struct, enum, interface, type alias, trait
- ✅ Включить doc comments
- ✅ Фильтровать по виду типа
- ✅ 90-95% экономия токенов

### Non-Goals
- ❌ Не включает функции
- ❌ Не включает impl blocks
- ❌ Не включает tests

---

## 🔧 API Specification

```json
{
  "name": "read_types_only",
  "description": "Извлечь только определения типов из файла. Экономит 90-95% токенов.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "kind": {
        "type": "string",
        "enum": ["struct", "enum", "interface", "type_alias", "trait"]
      },
      "include_docs": {"type": "boolean", "default": true}
    },
    "required": ["file"]
  }
}
```

---

## 💻 Implementation

Использует tree-sitter для фильтрации только type definition nodes (struct_item, enum_item, type_item, trait_item).

---

## 📈 Success Metrics

- ⚡ 90-95% экономия токенов
- ✅ 100% coverage type definitions
- ⏱️ Response time: < 500ms

---

## ✅ Acceptance Criteria

- [ ] Extracts all structs/enums/interfaces
- [ ] Includes doc comments
- [ ] Filter by kind works
- [ ] 90%+ token savings
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
