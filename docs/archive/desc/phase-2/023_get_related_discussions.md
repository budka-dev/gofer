# Feature: get_related_discussions - Контекст из GitHub

**ID:** PHASE2-023  
**Priority:** 🔥🔥🔥 High  
**Effort:** 5 дней  
**Status:** Not Started  
**Phase:** 2 (Human Context)

---

## 📋 Описание

Интеграция с GitHub для получения контекста обсуждений: PR, issues, code review comments связанные с конкретным кодом.

### Проблема

```
AI: "Почему этот код так написан?"
→ В PR было обсуждение, но оно не видно в коде

Developer: "Какие были проблемы с этой функцией?"
→ Issues не связаны с code location
```

### Решение

```typescript
const discussions = await gofer.get_related_discussions({
  file: "src/auth.rs",
  line: 120
});

// Returns:
// PR #234: "Add OAuth support"
//   - Comment by @alice: "Consider using library X"
//   - Decision: "Went with library Y for security"
// Issue #156: "Auth fails on edge case"
//   - Resolved in commit abc123
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ GitHub API integration (PR, issues, comments)
- ✅ Link code locations ↔ GitHub URLs
- ✅ Show resolved/unresolved discussions
- ✅ Extract decision context

### Non-Goals
- ❌ Не создает issues/PR
- ❌ Не модерирует discussions

---

## 🔧 API Specification

```json
{
  "name": "get_related_discussions",
  "description": "Найти PR/issues/comments связанные с кодом",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "line": {"type": "number", "description": "Optional"}
    },
    "required": ["file"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct Discussion {
    pub discussion_type: DiscussionType,
    pub number: u32,
    pub title: String,
    pub url: String,
    pub author: String,
    pub status: DiscussionStatus,
    pub comments: Vec<Comment>,
    pub related_commits: Vec<String>,
}

#[derive(Serialize)]
pub enum DiscussionType {
    PullRequest,
    Issue,
    CodeReview,
}

#[derive(Serialize)]
pub enum DiscussionStatus {
    Open,
    Closed,
    Merged,
}

#[derive(Serialize)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}
```

---

## 💻 Implementation

```rust
pub async fn get_related_discussions(
    file: &str,
    line: Option<u32>
) -> Result<Vec<Discussion>> {
    // 1. Get commits for this file
    let commits = get_commits_for_file(file).await?;
    
    let mut discussions = Vec::new();
    
    // 2. For each commit, find related PR/issues
    for commit in commits {
        // Query GitHub API
        let pr = find_pr_for_commit(&commit).await?;
        
        if let Some(pr) = pr {
            discussions.push(pr);
        }
        
        // Find issues mentioned in commit
        let issues = extract_issue_numbers(&commit.message);
        
        for issue_num in issues {
            let issue = fetch_issue(issue_num).await?;
            discussions.push(issue);
        }
    }
    
    Ok(discussions)
}

async fn fetch_issue(number: u32) -> Result<Discussion> {
    // gh api repos/:owner/:repo/issues/:number
    let output = Command::new("gh")
        .args(&["api", &format!("repos/owner/repo/issues/{}", number)])
        .output()?;
    
    let issue: GitHubIssue = serde_json::from_slice(&output.stdout)?;
    
    Ok(Discussion {
        discussion_type: DiscussionType::Issue,
        number: issue.number,
        title: issue.title,
        url: issue.html_url,
        author: issue.user.login,
        status: if issue.state == "closed" {
            DiscussionStatus::Closed
        } else {
            DiscussionStatus::Open
        },
        comments: vec![],
        related_commits: vec![],
    })
}
```

---

## 📈 Success Metrics

- ✅ Finds 80%+ relevant discussions
- ✅ Accurate linking code ↔ GitHub
- ⏱️ Response time < 3s

---

## ✅ Acceptance Criteria

- [ ] GitHub API integration works
- [ ] Finds related PR/issues
- [ ] Links to code locations
- [ ] Shows comments
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
