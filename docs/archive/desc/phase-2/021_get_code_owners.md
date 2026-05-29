# Feature: get_code_owners - Эксперты кодовой базы

**ID:** PHASE2-021  
**Priority:** 🔥🔥 Medium  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 2 (Human Context)

---

## 📋 Описание

Определение code owners - экспертов в конкретных модулях на основе git history. Показывает кто лучше всего знает этот код и к кому обратиться с вопросами.

### Проблема

```
AI: "Кто эксперт по auth модулю?"
→ Без анализа git history непонятно к кому обратиться

Developer: "Нужно code review для payment.rs"
→ Не знаем кто лучше всего знает этот код
```

### Решение

```typescript
const owners = await gofer.get_code_owners({
  file: "src/auth/mod.rs"
});

// Returns:
// Primary: @alice (68% commits, expert)
// Secondary: @bob (22% commits)
// Contributors: @charlie (10%)
// Contact: alice@company.com
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Identify code experts via git history
- ✅ Calculate contribution percentages
- ✅ Parse CODEOWNERS file
- ✅ Provide contact information
- ✅ Rank by expertise level

### Non-Goals
- ❌ Не automatic reviewer assignment
- ❌ Не team management

---

## 🔧 API Specification

```json
{
  "name": "get_code_owners",
  "description": "Найти экспертов модуля по git history",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "since": {"type": "string", "default": "1 year ago"}
    },
    "required": ["file"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct CodeOwner {
    pub name: String,
    pub email: String,
    pub commit_count: usize,
    pub contribution_percent: f32,
    pub expertise_level: ExpertiseLevel,
    pub last_commit: DateTime<Utc>,
}

#[derive(Serialize)]
pub enum ExpertiseLevel {
    Expert,      // > 50% commits
    Contributor, // 20-50%
    Minor,       // < 20%
}
```

---

## 💻 Implementation

```rust
pub async fn get_code_owners(file: &str) -> Result<Vec<CodeOwner>> {
    // 1. Git log для файла
    let output = Command::new("git")
        .args(&["log", "--follow", "--format=%an|%ae|%ad", "--", file])
        .output()?;
    
    // 2. Aggregate по авторам
    let mut authors: HashMap<String, AuthorStats> = HashMap::new();
    
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 2 {
            let entry = authors.entry(parts[0].to_string()).or_default();
            entry.commit_count += 1;
            entry.email = parts[1].to_string();
        }
    }
    
    // 3. Calculate percentages
    let total_commits = authors.values().map(|a| a.commit_count).sum::<usize>();
    
    let mut owners: Vec<CodeOwner> = authors.into_iter()
        .map(|(name, stats)| {
            let percent = (stats.commit_count as f32 / total_commits as f32) * 100.0;
            let level = match percent {
                p if p > 50.0 => ExpertiseLevel::Expert,
                p if p > 20.0 => ExpertiseLevel::Contributor,
                _ => ExpertiseLevel::Minor,
            };
            
            CodeOwner {
                name,
                email: stats.email,
                commit_count: stats.commit_count,
                contribution_percent: percent,
                expertise_level: level,
                last_commit: Utc::now(), // TODO: parse from git
            }
        })
        .collect();
    
    // 4. Sort by contribution
    owners.sort_by(|a, b| {
        b.contribution_percent.partial_cmp(&a.contribution_percent).unwrap()
    });
    
    Ok(owners)
}
```

---

## 📈 Success Metrics

- ✅ Identifies primary owner (highest contributor)
- ✅ Accurate contribution percentages
- ⏱️ Response time < 2s

---

## ✅ Acceptance Criteria

- [ ] Parses git history
- [ ] Calculates contributions
- [ ] Ranks by expertise
- [ ] Includes contact info
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
