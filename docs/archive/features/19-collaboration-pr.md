# 19. Collaboration Layer (Pull Requests & Code Review)

## Категория
Работа в команде

## Приоритет
🔴 **P1** (Очень полезно)

## Оценка полезности для AI
⭐⭐⭐⭐⭐ (5/5)

## Описание
Интеграция с GitHub/GitLab для создания PR, чтения code review комментариев и разрешения merge conflicts.

## Проблема
AI работает в изоляции. Он не может создать Pull Request, прочитать комментарии ревьюера или разрешить merge conflicts. Это делает AI "одиночкой", а не членом команды.

## API

### create_draft_pr(title, description, base_branch)
Создать draft Pull Request.

**Параметры:**
- `title` (string) — заголовок PR
- `description` (string) — описание (Markdown)
- `base_branch` (string, optional) — базовая ветка (по умолчанию: main/master)

**Возвращает:**
```json
{
  "pr_number": 123,
  "url": "https://github.com/user/repo/pull/123",
  "status": "draft",
  "branch": "feature/refactor-auth",
  "base_branch": "main",
  "commits": 5,
  "files_changed": 12
}
```

**Пример:**
```
AI: (завершил рефакторинг)
AI: create_draft_pr(
  title="Refactor authentication module",
  description="## Summary\n- Extracted auth logic into separate module\n- Added JWT support\n- Improved error handling\n\n## Test plan\n- [x] Unit tests pass\n- [x] Integration tests pass\n- [ ] Manual testing needed",
  base_branch="main"
)

Result: Draft PR #123 created
AI: "Pull Request создан: https://github.com/user/repo/pull/123"
```

---

### get_pr_comments(pr_number)
Получить комментарии code review.

**Параметры:**
- `pr_number` (number) — номер PR

**Возвращает:**
```json
{
  "pr_number": 123,
  "comments": [
    {
      "id": "comment_001",
      "author": "reviewer@example.com",
      "file": "src/auth.rs",
      "line": 45,
      "body": "Please add error handling here",
      "created_at": "2024-02-20T10:30:00Z",
      "resolved": false
    },
    {
      "id": "comment_002",
      "author": "lead@example.com",
      "file": "src/api.rs",
      "line": 120,
      "body": "Consider using match instead of if-else chain",
      "created_at": "2024-02-20T11:00:00Z",
      "resolved": false
    }
  ],
  "total_comments": 2,
  "unresolved_comments": 2
}
```

**Пример:**
```
AI: get_pr_comments(123)
Result: 2 unresolved comments

AI: (анализирует комментарии)
AI: patch_file("src/auth.rs", line=45, ...) # добавляет error handling
AI: patch_file("src/api.rs", line=120, ...) # заменяет на match
AI: add_review_comment("src/auth.rs", 45, "Added error handling as requested")
```

---

### apply_pr_suggestion(pr_number, suggestion_id)
Применить предложение из code review.

**Параметры:**
- `pr_number` (number) — номер PR
- `suggestion_id` (string) — ID suggestion block

**GitHub suggestions** — это блоки кода в комментариях:
```markdown
```suggestion
let result = match value {
    Some(v) => v,
    None => return Err(Error::Missing),
};
\```
```

**Возвращает:**
```json
{
  "pr_number": 123,
  "suggestion_id": "sugg_001",
  "status": "applied",
  "file": "src/auth.rs",
  "commit": "abc123"
}
```

**Пример:**
```
AI: get_pr_comments(123)
Result: reviewer предложил suggestion для src/auth.rs

AI: apply_pr_suggestion(123, "sugg_001")
Result: suggestion применён автоматически
```

---

### resolve_merge_conflict(path, strategy)
Интеллектуально разрешить merge conflict.

**Параметры:**
- `path` (string) — путь к файлу с конфликтом
- `strategy` (string) — стратегия разрешения
  - `"accept_ours"` — принять нашу версию
  - `"accept_theirs"` — принять их версию
  - `"smart_merge"` — AI анализирует и объединяет оба изменения

**Возвращает:**
```json
{
  "path": "src/auth.rs",
  "strategy": "smart_merge",
  "status": "resolved",
  "conflicts_found": 3,
  "conflicts_resolved": 3,
  "resolution": "AI merged both changes: kept new function signature from 'ours' and error handling from 'theirs'"
}
```

**Пример конфликта:**
```
<<<<<<< ours
fn authenticate(token: &str) -> Result<User> {
    verify_token(token)
}
=======
fn login(token: String) -> User {
    verify_token(&token).unwrap()
}
>>>>>>> theirs
```

**AI с strategy="smart_merge":**
```rust
fn authenticate(token: &str) -> Result<User> {
    verify_token(token)  // Взял signature из ours, но оставил Result
}
```

---

### add_review_comment(path, line, text)
AI оставляет комментарий для себя или ревьюера.

**Параметры:**
- `path` (string) — путь к файлу
- `line` (number) — строка
- `text` (string) — текст комментария

**Возвращает:**
```json
{
  "comment_id": "comment_123",
  "path": "src/auth.rs",
  "line": 45,
  "text": "Added error handling as requested",
  "url": "https://github.com/user/repo/pull/123#discussion_r456"
}
```

---

### mark_pr_ready(pr_number)
Перевести draft PR в ready for review.

**Параметры:**
- `pr_number` (number)

**Возвращает:**
```json
{
  "pr_number": 123,
  "status": "ready_for_review",
  "reviewers_requested": ["lead@example.com", "reviewer@example.com"]
}
```

## Примеры комплексного использования

### Сценарий 1: Полный цикл PR
```
User: "Отрефактори auth модуль и создай PR"

AI: (делает рефакторинг)
AI: create_draft_pr("Refactor auth module", "...")
Result: PR #123 created (draft)

AI: (получает code review)
AI: get_pr_comments(123)
Result: 3 comments от ревьюера

AI: (фиксит замечания)
AI: patch_file(...)
AI: add_review_comment("src/auth.rs", 45, "Fixed as requested")

AI: mark_pr_ready(123)
Result: PR ready for review

User: "Approve"
AI: "PR готов к мержу!"
```

### Сценарий 2: Разрешение конфликтов
```
AI: (пытается смержить ветку)
Error: merge conflict in src/api.rs

AI: resolve_merge_conflict("src/api.rs", strategy="smart_merge")
Result: {status: "resolved"}

AI: "Конфликт разрешён: объединены изменения из обеих веток"
```

## Преимущества

### 1. AI как полноценный член команды
AI создаёт PR, реагирует на код ревью, общается через комментарии.

### 2. Автоматизация рутины
Применение reviewer suggestions, разрешение простых конфликтов.

### 3. Быстрая итерация
AI моментально фиксит замечания ревьюера.

## Сложность реализации
Высокая (7-10 дней)
- GitHub API integration: средняя (3 дня)
- GitLab support: средняя (2 дня)
- Smart merge conflicts: высокая (5 дней)

## Статус в gofer
❌ Отсутствует

## Зависимости
- GitHub API / GitLab API
- git2 library
- AST parser (для smart merge)

## Безопасность

### OAuth токены
```toml
[collaboration]
github_token = "$GITHUB_TOKEN"  # env variable
gitlab_token = "$GITLAB_TOKEN"
```

### Permissions
- Создание PR: требует write access
- Merge: требует подтверждение пользователя

## Конфигурация

```toml
[collaboration]
# Автоматически создавать draft PR после больших рефакторингов
auto_create_pr = false

# Автоматически применять reviewer suggestions
auto_apply_suggestions = false

# Стратегия разрешения конфликтов по умолчанию
default_conflict_strategy = "smart_merge"  # "smart_merge" | "accept_ours" | "accept_theirs"

# Запрашивать подтверждение пользователя перед merge
require_merge_confirmation = true
```

## Альтернативы
- Пользователь вручную создаёт PR (медленно)
- AI только генерирует код, не участвует в review (неполно)
- Внешний GitHub MCP (может не иметь AI-powered merge)

## Связанные функции
- `git_history` — анализ истории для PR description
- `run_all_tests` — запустить тесты перед созданием PR
- `format_file` — причесать код перед PR
