# Feature: get_code_evolution - История изменений кода

**ID:** PHASE1-012  
**Priority:** 🔥🔥🔥 High  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context)

---

## 📋 Описание

MCP tool для получения истории изменений файла/функции через git history. Показывает эволюцию кода во времени: кто менял, когда, почему.

### Проблема

**AI не видит историю:**
```
AI: "Почему эта функция так сложно написана?"
→ Без истории не понять, что это legacy код с накопленными workarounds

AI: "Кто автор этой логики?"
→ Без git blame непонятно к кому обратиться

AI: "Когда это сломалось?"
→ Без истории невозможно найти момент регрессии
```

### Решение

```typescript
const evolution = await gofer.get_code_evolution({
  file: "src/auth.rs",
  function: "authenticate",
  since: "6 months ago"
});

// Returns:
// - 12 commits изменивших эту функцию
// - Авторы: @alice (8), @bob (4)
// - Типы изменений: bugfix (6), feature (4), refactor (2)
// - Churn rate: High (много изменений = нестабильный код)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ История изменений файла/функции
- ✅ Git blame по строкам
- ✅ Churn analysis (частота изменений)
- ✅ Авторы и commit messages
- ✅ Интеграция с find_hotspots

### Non-Goals
- ❌ Не полный git log (только релевантные изменения)
- ❌ Не diff visualization (только метаданные)

---

## 🔧 API Specification

```json
{
  "name": "get_code_evolution",
  "description": "История изменений файла/функции через git",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "function": {"type": "string", "description": "Optional: конкретная функция"},
      "since": {"type": "string", "description": "Git date format (e.g., '6 months ago')"},
      "limit": {"type": "number", "default": 20}
    },
    "required": ["file"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct CodeEvolution {
    pub commits: Vec<CommitInfo>,
    pub authors: HashMap<String, usize>,  // author -> commit count
    pub churn_rate: ChurnRate,
    pub hotspots: Vec<LineRange>,
}

#[derive(Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub message: String,
    pub change_type: ChangeType,  // Feature, Bugfix, Refactor
    pub lines_added: usize,
    pub lines_removed: usize,
}

#[derive(Serialize)]
pub enum ChurnRate {
    Low,      // < 5 изменений за период
    Medium,   // 5-15 изменений
    High,     // > 15 изменений (potential problem area)
}
```

---

## 💻 Implementation

```rust
pub async fn handle_get_code_evolution(
    args: &Map<String, Value>
) -> Result<Value> {
    let file = args.get("file").unwrap().as_str().unwrap();
    let since = args.get("since")
        .and_then(|v| v.as_str())
        .unwrap_or("1 year ago");
    
    // 1. Get git log for file
    let output = Command::new("git")
        .args(&["log", "--follow", "--since", since, "--", file])
        .output()?;
    
    // 2. Parse commits
    let commits = parse_git_log(&output.stdout)?;
    
    // 3. Calculate churn
    let churn_rate = calculate_churn_rate(&commits);
    
    // 4. Find hotspots (most changed lines)
    let hotspots = find_hotspots_in_file(file).await?;
    
    Ok(json!({
        "commits": commits,
        "churn_rate": churn_rate,
        "hotspots": hotspots
    }))
}
```

---

## 📈 Success Metrics

- ✅ Accurate git history extraction
- ⏱️ Response time < 3s
- 📊 Churn analysis helps identify problem areas

---

## ✅ Acceptance Criteria

- [ ] Extracts git history for file
- [ ] Supports function-level history
- [ ] Calculates churn rate
- [ ] Identifies hotspots
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
