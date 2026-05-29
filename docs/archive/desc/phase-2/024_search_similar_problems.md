# Feature: search_similar_problems - Поиск похожих проблем

**ID:** PHASE2-024  
**Priority:** 🔥🔥 Medium  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 2 (Human Context)

---

## 📋 Описание

Semantic search по историческим issues для поиска похожих проблем. Показывает какие решения работали/не работали в прошлом.

### Проблема

```
Developer: "Auth fails intermittently"
→ Похожая проблема была решена в Issue #234, но не знаем об этом

AI: "How to implement feature X?"
→ Есть related issues с обсуждением, но semantic search их не найдет по keywords
```

### Решение

```typescript
const similar = await gofer.search_similar_problems({
  description: "Authentication fails randomly under load"
});

// Returns:
// Issue #234: "Auth timeout under high traffic" (90% similarity)
//   Solution: Added connection pooling
//   Status: Resolved
// Issue #156: "Random auth failures" (75% similarity)
//   Solution: Fixed race condition
//   Status: Resolved
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Semantic search по issue descriptions
- ✅ Vector embeddings для issues
- ✅ Show solutions that worked
- ✅ Rank by similarity

### Non-Goals
- ❌ Не создает issues
- ❌ Не automatic problem solving

---

## 🔧 API Specification

```json
{
  "name": "search_similar_problems",
  "description": "Найти похожие проблемы в issue history",
  "inputSchema": {
    "type": "object",
    "properties": {
      "description": {"type": "string"},
      "limit": {"type": "number", "default": 10},
      "status": {
        "type": "string",
        "enum": ["all", "open", "closed"],
        "default": "all"
      }
    },
    "required": ["description"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct HistoricalIssue {
    pub number: u32,
    pub title: String,
    pub description: String,
    pub similarity_score: f32,
    pub status: String,
    pub solution: Option<String>,
    pub related_commits: Vec<String>,
    pub url: String,
}
```

---

## 💻 Implementation

```rust
pub async fn search_similar_problems(
    description: &str,
    limit: usize
) -> Result<Vec<HistoricalIssue>> {
    // 1. Embed query
    let query_embedding = embed_text(description).await?;
    
    // 2. Vector search in LanceDB
    let results = lance_db.search_vectors(query_embedding, limit).await?;
    
    // 3. Fetch issue details
    let mut issues = Vec::new();
    
    for result in results {
        let issue = fetch_issue_by_id(result.id).await?;
        
        issues.push(HistoricalIssue {
            number: issue.number,
            title: issue.title,
            description: issue.body,
            similarity_score: result.score,
            status: issue.state,
            solution: extract_solution(&issue),
            related_commits: find_related_commits(&issue).await?,
            url: issue.html_url,
        });
    }
    
    Ok(issues)
}

fn extract_solution(issue: &GitHubIssue) -> Option<String> {
    // Parse comments for solution
    // Look for patterns: "Fixed by", "Solution:", etc.
    
    todo!()
}
```

---

## 📈 Success Metrics

- ✅ 70%+ relevant results
- ✅ Similarity scores accurate
- ⏱️ Response time < 2s

---

## ✅ Acceptance Criteria

- [ ] Semantic search works
- [ ] Issues indexed with embeddings
- [ ] Solutions extracted
- [ ] Similarity ranking accurate
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
