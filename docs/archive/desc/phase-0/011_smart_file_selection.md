# Feature: smart_file_selection - Умный выбор файлов для чтения

**ID:** PHASE0-011  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)

---

## 📋 Описание

AI assistant который помогает LLM выбрать правильные файлы для чтения на основе естественного запроса. Вместо того чтобы читать все подряд, AI получает рекомендацию какие именно файлы нужны для задачи.

### Проблема

**Сценарий: "Как работает аутентификация?"**

```
Без smart_file_selection:
AI пробует:
1. read_file("src/main.rs") - 500 строк, нет auth
2. search("authentication") - 50 результатов, слишком много
3. read_file("src/server.rs") - 800 строк, есть немного
4. read_file("src/auth.rs") - Наконец-то! Но потратили 3 запроса
5. read_file("src/middleware/jwt.rs") - Тоже нужен был

Total: 5 запросов, ~3000 строк, 10+ секунд
```

**С smart_file_selection:**
```
AI: smart_file_selection("Как работает аутентификация?")

Ответ:
Relevance Score:
1. src/auth/mod.rs (95%) - main auth logic
2. src/auth/jwt.rs (90%) - JWT token handling  
3. src/middleware/auth.rs (85%) - auth middleware
4. src/models/user.rs (70%) - User model with roles
5. config/auth.yaml (60%) - auth configuration

AI сразу читает топ-3 файла
Total: 1 recommendation + 3 reads, ~800 строк, 3 секунды
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Ранжировать файлы по релевантности к запросу
- ✅ Использовать file summaries + vector search
- ✅ Учитывать file names, paths, symbols
- ✅ Рекомендовать топ-N файлов
- ✅ 70%+ accuracy (правильный файл в топ-3)

### Non-Goals
- ❌ Не заменяет search (дополняет его)
- ❌ Не читает файлы автоматически
- ❌ Не гарантирует 100% точность

---

## 🏗️ Архитектура

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│    smart_file_selection()               │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Query Analyzer │
        │  (understand    │
        │   intent)       │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬────────────┐
     │           │           │            │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐ ┌───▼────┐
│ Vector   │ │Symbol│ │   Path    │ │Summary │
│ Search   │ │Index │ │  Matcher  │ │ Ranker │
└──┬───────┘ └──┬───┘ └────┬──────┘ └───┬────┘
   │            │           │            │
   └────────────┴───────────┴────────────┘
                     │
            ┌────────▼────────┐
            │ Score Aggregator│
            │  (ML-based)     │
            └────────┬────────┘
                     │
            ┌────────▼────────┐
            │ Ranked File List│
            └─────────────────┘
```

---

## 📊 Data Model

### MCP Tool Definition

```json
{
  "name": "smart_file_selection",
  "description": "Получить ранжированный список файлов релевантных для задачи. Помогает AI выбрать что читать.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Описание задачи или вопроса"
      },
      "limit": {
        "type": "number",
        "default": 5,
        "description": "Сколько файлов вернуть"
      },
      "min_score": {
        "type": "number",
        "default": 0.3,
        "description": "Минимальный score релевантности (0-1)"
      }
    },
    "required": ["query"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct FileSelectionResponse {
    pub files: Vec<RankedFile>,
    pub reasoning: String,
    pub total_candidates: usize,
}

#[derive(Serialize)]
pub struct RankedFile {
    pub path: String,
    pub score: f32,
    pub reason: String,
    pub summary: Option<String>,
    pub key_symbols: Vec<String>,
}
```

---

## 💻 Implementation Strategy

### Scoring Algorithm

```rust
fn calculate_relevance_score(
    query: &str,
    file: &FileInfo,
    vector_score: f32,
    symbol_matches: &[String],
) -> f32 {
    let mut score = 0.0;
    
    // 1. Vector similarity (40% weight)
    score += vector_score * 0.4;
    
    // 2. Path matching (20% weight)
    let path_score = calculate_path_score(query, &file.path);
    score += path_score * 0.2;
    
    // 3. Symbol matches (25% weight)
    let symbol_score = calculate_symbol_score(query, symbol_matches);
    score += symbol_score * 0.25;
    
    // 4. Summary relevance (15% weight)
    if let Some(ref summary) = file.summary {
        let summary_score = calculate_summary_score(query, summary);
        score += summary_score * 0.15;
    }
    
    score
}
```

---

## 📈 Success Metrics

### Accuracy
- ✅ 70%+ top-3 accuracy (нужный файл в топ-3)
- ✅ 90%+ top-5 accuracy
- ✅ < 5% false positives

### Performance
- ⏱️ Response time: < 2s
- 📊 Process 1000+ files efficiently

---

## 📚 Usage Examples

```typescript
// AI понимает что пользователь спрашивает про auth
const result = await gofer.smart_file_selection({
  query: "Как работает JWT authentication?",
  limit: 5
});

// AI получает топ-5 файлов
result.files.forEach(file => {
  console.log(`${file.path} (${file.score}) - ${file.reason}`);
});

// AI решает прочитать топ-3
for (const file of result.files.slice(0, 3)) {
  const content = await gofer.read_file({ file: file.path });
  // analyze...
}
```

---

## ✅ Acceptance Criteria

- [ ] Ранжирует файлы по релевантности
- [ ] Использует vector search + metadata
- [ ] 70%+ top-3 accuracy
- [ ] Response time < 2s
- [ ] Reasoning объясняет выбор
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** КРИТИЧЕСКИЙ - это ключевой инструмент для эффективной навигации в больших кодбазах.
