# Smart Commit Design - Автоматическая генерация коммитов

> **Context:** Дизайн-документ для фичи автоматического создания осмысленных git коммитов на основе анализа изменений.
> 
> **Goal:** Убрать рутину написания commit messages, сохраняя качество и безопасность.

**Date:** 2026-02-16  
**Status:** Design Proposal

---

## 🎯 Проблема

**Текущий workflow:**
```bash
# 1. Пишешь код
$ vim src/daemon/tools.rs

# 2. Смотришь что изменилось
$ git diff

# 3. Думаешь что написать в коммите
$ git add .
$ git commit -m "???"  # Что писать?

# 4. Пишешь generic message
$ git commit -m "update tools"  # Не информативно
$ git commit -m "add new features and fix bugs"  # Слишком общее
```

**Pain points:**
- Тратишь время на формулировку
- Коммиты получаются generic ("update", "fix", "wip")
- Забываешь детали того что делал
- Приходится перечитывать diff
- Нет consistency в стиле

---

## 💡 Решение: Smart Commit

gofer анализирует изменения и генерирует осмысленный коммит автоматически.

### Три режима работы:

#### 1️⃣ **Suggest Mode** (безопасный, по умолчанию)
Предлагает коммит, ждёт подтверждения.

```
User: smart_commit

gofer: 📝 Предлагаю коммит:
  
  feat(roadmap): add extended roadmap with community insights ✨
  
  - Create ROADMAP_EXTENSIONS.md with 7 new feature proposals
  - Focus on token efficiency, real-time assistance, UX improvements
  - Mark quick wins: token-efficient reading, change impact analysis
  
  Files (1):
  + ROADMAP_EXTENSIONS.md (15.2 KB, new file)
  
  Quality checks:
  ✅ No compilation errors
  ✅ No sensitive files
  ✅ Reasonable size (1 file)
  
  [✅ Approve] [✏️ Edit] [❌ Cancel]

User: approve

gofer: ✅ Committed: a3f5b21
  View: git show a3f5b21
```

#### 2️⃣ **Auto Mode** (делает сразу)
Коммитит без подтверждения, показывает что сделано.

```
User: smart_commit --auto

gofer: ✅ Auto-committed: a3f5b21
  
  feat(roadmap): add extended roadmap with community insights ✨
  
  Files: ROADMAP_EXTENSIONS.md
  
  💡 Tip: git reset HEAD^ чтобы откатить
```

#### 3️⃣ **Watch Mode** (фоновый)
Автоматически коммитит по триггерам.

```
User: smart_commit --watch --trigger=on-save

gofer: 🔄 Auto-commit watch enabled
  Trigger: on file save
  Will create micro-commits automatically
  
  💡 Используйте git rebase -i для squash позже

[... вы редактируете файлы ...]

gofer: ✅ Auto-committed: b4c8d32 (file saved)
gofer: ✅ Auto-committed: e7f1a54 (file saved)
gofer: ✅ Auto-committed: 3a9d6b1 (file saved)

User: smart_commit --watch-stop

gofer: ⏸️  Auto-commit watch disabled
  Created 3 commits, squash? [Yes] [No]
```

---

## 🏗️ Архитектура

### Компоненты:

```
┌─────────────────────────────────────────────────────┐
│                   MCP Tools                         │
├─────────────────────────────────────────────────────┤
│  suggest_commit()   auto_commit()   watch_mode()   │
└────────────────┬────────────────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Smart Commit   │
        │     Engine      │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│ Analyzer │ │ Gen  │ │  Safety   │
│  Module  │ │Module│ │  Checker  │
└────┬─────┘ └──┬───┘ └────┬──────┘
     │          │           │
     │          │           │
┌────▼──────────▼───────────▼──────┐
│         Git Integration          │
└──────────────────────────────────┘
```

### 1. Analyzer Module (Анализ изменений)

```rust
struct ChangeAnalysis {
    // Что изменилось
    modified_files: Vec<FileChange>,
    added_files: Vec<String>,
    deleted_files: Vec<String>,
    renamed_files: Vec<(String, String)>,
    
    // Semantic changes
    added_functions: Vec<Symbol>,
    modified_functions: Vec<Symbol>,
    deleted_functions: Vec<Symbol>,
    
    // Scope
    scope: ChangeScope,  // Single file, Module, Multi-module, Architecture
    
    // Type
    change_type: ChangeType,  // Feature, Fix, Refactor, Docs, etc.
    
    // Context
    related_issues: Vec<String>,  // "Closes #123"
    breaking_changes: Vec<BreakingChange>,
}

impl ChangeAnalysis {
    fn from_git_diff(diff: &str) -> Result<Self> {
        // Parse git diff
        // Extract file changes
        // Analyze symbols (через tree-sitter)
        // Classify change type
    }
    
    fn detect_change_type(&self) -> ChangeType {
        // Heuristics:
        // - New files + exports → Feature
        // - Modified existing + tests → Fix
        // - Renamed/moved → Refactor
        // - Only comments/docs → Docs
        // - Tests only → Test
    }
    
    fn detect_scope(&self) -> String {
        // Пример: "roadmap", "daemon/tools", "indexer"
        // Используется для conventional commits
    }
}

enum ChangeType {
    Feature,      // feat:
    Fix,          // fix:
    Refactor,     // refactor:
    Docs,         // docs:
    Test,         // test:
    Chore,        // chore:
    Perf,         // perf:
    Style,        // style:
}
```

### 2. Generator Module (Генерация сообщений)

```rust
struct MessageGenerator {
    style: CommitStyle,
    project_conventions: ProjectConventions,
}

enum CommitStyle {
    Conventional,  // feat(scope): subject
    Simple,        // Subject only
    Detailed,      // Subject + body with bullets
}

struct ProjectConventions {
    use_emoji: bool,
    max_subject_length: usize,  // 50, 72, unlimited
    language: Language,  // EN, RU
    conventional_commits: bool,
    emoji_map: HashMap<ChangeType, String>,
}

impl MessageGenerator {
    fn generate(&self, analysis: &ChangeAnalysis) -> CommitMessage {
        let subject = self.generate_subject(analysis);
        let body = self.generate_body(analysis);
        
        CommitMessage { subject, body }
    }
    
    fn generate_subject(&self, analysis: &ChangeAnalysis) -> String {
        let prefix = match self.style {
            Conventional => {
                let type_str = analysis.change_type.as_str();
                let scope = analysis.detect_scope();
                format!("{type_str}({scope})")
            },
            _ => String::new(),
        };
        
        let emoji = if self.project_conventions.use_emoji {
            self.emoji_for_type(&analysis.change_type)
        } else {
            String::new()
        };
        
        let description = self.summarize_changes(analysis);
        
        format!("{prefix}: {description} {emoji}").trim().to_string()
    }
    
    fn generate_body(&self, analysis: &ChangeAnalysis) -> Option<String> {
        if analysis.is_simple() {
            return None;  // Короткие изменения не нуждаются в body
        }
        
        let mut lines = Vec::new();
        
        // Добавляем bullets для основных изменений
        for file in &analysis.modified_files {
            if let Some(summary) = file.summarize() {
                lines.push(format!("- {}", summary));
            }
        }
        
        // Breaking changes (если есть)
        if !analysis.breaking_changes.is_empty() {
            lines.push("\nBREAKING CHANGE:".to_string());
            for bc in &analysis.breaking_changes {
                lines.push(format!("- {}", bc.description));
            }
        }
        
        // Related issues
        for issue in &analysis.related_issues {
            lines.push(format!("\nCloses {}", issue));
        }
        
        Some(lines.join("\n"))
    }
    
    fn learn_from_history(&mut self, repo: &Repository) -> Result<()> {
        // Анализирует последние N коммитов
        // Определяет паттерны:
        // - Используют ли emoji?
        // - Conventional commits?
        // - Средняя длина subject
        // - Язык (EN/RU)
        
        let recent = repo.log(limit: 20)?;
        
        self.project_conventions.use_emoji = 
            recent.iter().any(|c| contains_emoji(&c.message));
        
        self.project_conventions.conventional_commits =
            recent.iter().filter(|c| is_conventional(&c.message)).count() > 10;
        
        // и т.д.
        
        Ok(())
    }
}

struct CommitMessage {
    subject: String,
    body: Option<String>,
}

impl CommitMessage {
    fn format(&self) -> String {
        match &self.body {
            Some(body) => format!("{}\n\n{}", self.subject, body),
            None => self.subject.clone(),
        }
    }
}
```

### 3. Safety Checker (Проверки безопасности)

```rust
struct SafetyChecker {
    strict_mode: bool,
}

struct SafetyReport {
    can_commit: bool,
    errors: Vec<SafetyError>,
    warnings: Vec<SafetyWarning>,
}

enum SafetyError {
    CompilationErrors(Vec<String>),
    SecretFilesStaged(Vec<String>),
    DetachedHead,
    MergeInProgress,
    RebaseInProgress,
}

enum SafetyWarning {
    LargeCommit { files: usize, threshold: usize },
    NoTests,
    UnresolvedComments,  // TODO/FIXME в staged файлах
    LongSubject { length: usize, max: usize },
}

impl SafetyChecker {
    fn check(&self, analysis: &ChangeAnalysis) -> SafetyReport {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // 1. Проверка компиляции
        if let Err(e) = self.check_compilation() {
            errors.push(SafetyError::CompilationErrors(e));
        }
        
        // 2. Проверка секретов
        let secrets = self.detect_secrets(&analysis.modified_files);
        if !secrets.is_empty() {
            errors.push(SafetyError::SecretFilesStaged(secrets));
        }
        
        // 3. Проверка Git состояния
        if self.is_detached_head()? {
            errors.push(SafetyError::DetachedHead);
        }
        
        // 4. Предупреждения
        if analysis.modified_files.len() > 10 {
            warnings.push(SafetyWarning::LargeCommit {
                files: analysis.modified_files.len(),
                threshold: 10,
            });
        }
        
        if !self.has_tests(&analysis) {
            warnings.push(SafetyWarning::NoTests);
        }
        
        SafetyReport {
            can_commit: errors.is_empty(),
            errors,
            warnings,
        }
    }
    
    fn detect_secrets(&self, files: &[FileChange]) -> Vec<String> {
        let secret_patterns = [
            ".env",
            "credentials.json",
            "*.key",
            "*.pem",
            "*_rsa",
            "*.p12",
        ];
        
        let secret_content_patterns = [
            r"password\s*=\s*['\"].*['\"]",
            r"api_key\s*=\s*['\"].*['\"]",
            r"token\s*=\s*['\"].*['\"]",
            r"private_key",
            r"-----BEGIN (RSA )?PRIVATE KEY-----",
        ];
        
        // Проверяем и имена файлов и контент
        files.iter()
            .filter(|f| {
                self.matches_pattern(&f.path, &secret_patterns) ||
                self.contains_pattern(&f.content, &secret_content_patterns)
            })
            .map(|f| f.path.clone())
            .collect()
    }
    
    fn check_compilation(&self) -> Result<(), Vec<String>> {
        // Зависит от языка проекта
        // Rust: cargo check
        // TypeScript: tsc --noEmit
        // Python: mypy
        
        // Используем существующий get_errors() из gofer
    }
}
```

---

## 🔧 MCP Tools API

### Tool 1: `suggest_commit`

```json
{
  "name": "suggest_commit",
  "description": "Анализирует uncommitted изменения и предлагает commit message",
  "inputSchema": {
    "type": "object",
    "properties": {
      "files": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Конкретные файлы для коммита (если null - все staged/modified)"
      },
      "style": {
        "type": "string",
        "enum": ["conventional", "simple", "detailed"],
        "default": "conventional",
        "description": "Стиль commit message"
      },
      "include_emoji": {
        "type": "boolean",
        "default": true,
        "description": "Добавлять ли emoji в subject"
      }
    }
  }
}
```

**Response:**
```json
{
  "suggested_message": {
    "subject": "feat(roadmap): add extended roadmap with community insights ✨",
    "body": "- Create ROADMAP_EXTENSIONS.md with 7 new feature proposals\n- Focus on token efficiency and real-time assistance\n- Mark quick wins for immediate implementation"
  },
  "files": [
    { "path": "ROADMAP_EXTENSIONS.md", "status": "added", "size": 15573 }
  ],
  "analysis": {
    "change_type": "feature",
    "scope": "roadmap",
    "complexity": "medium"
  },
  "safety_check": {
    "can_commit": true,
    "warnings": []
  }
}
```

### Tool 2: `auto_commit`

```json
{
  "name": "auto_commit",
  "description": "Создаёт коммит автоматически на основе анализа изменений",
  "inputSchema": {
    "type": "object",
    "properties": {
      "files": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Файлы для коммита (null = все)"
      },
      "style": {
        "type": "string",
        "enum": ["conventional", "simple", "detailed"],
        "default": "conventional"
      },
      "force": {
        "type": "boolean",
        "default": false,
        "description": "Игнорировать warnings (НЕ errors)"
      },
      "dry_run": {
        "type": "boolean",
        "default": false,
        "description": "Показать что будет сделано без реального коммита"
      }
    }
  }
}
```

**Response:**
```json
{
  "commit_hash": "a3f5b21c8d4e9f0a1b2c3d4e5f6a7b8c9d0e1f2a",
  "message": "feat(roadmap): add extended roadmap ✨\n\n- Create ROADMAP_EXTENSIONS.md...",
  "files_committed": ["ROADMAP_EXTENSIONS.md"],
  "stats": {
    "insertions": 547,
    "deletions": 0,
    "files_changed": 1
  },
  "can_undo": true,
  "undo_command": "git reset HEAD^"
}
```

### Tool 3: `commit_watch`

```json
{
  "name": "commit_watch",
  "description": "Управление фоновым авто-коммитом",
  "inputSchema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "enum": ["start", "stop", "status"],
        "description": "Действие с watch mode"
      },
      "trigger": {
        "type": "string",
        "enum": ["on_save", "on_test_pass", "periodic", "on_clean_build"],
        "default": "on_save",
        "description": "Когда создавать авто-коммиты"
      },
      "interval_minutes": {
        "type": "integer",
        "default": 15,
        "description": "Интервал для periodic trigger"
      },
      "squash_on_stop": {
        "type": "boolean",
        "default": true,
        "description": "Предложить squash при остановке"
      }
    },
    "required": ["action"]
  }
}
```

**Response (start):**
```json
{
  "status": "watching",
  "trigger": "on_save",
  "commits_created": 0,
  "watching_files": ["*.rs", "*.md", "*.toml"]
}
```

**Response (stop):**
```json
{
  "status": "stopped",
  "commits_created": 5,
  "commits": [
    "a3f5b21: feat: add feature X",
    "b4c8d32: fix: handle edge case",
    "e7f1a54: refactor: simplify logic",
    "3a9d6b1: test: add unit tests",
    "9c2e5f3: docs: update README"
  ],
  "suggest_squash": true,
  "squash_command": "git rebase -i HEAD~5"
}
```

---

## 📝 Примеры использования

### Сценарий 1: Простой коммит

```
# Редактируете файл
$ vim src/daemon/tools.rs

# Готовы закоммитить
User: suggest_commit

gofer: 📝 Анализирую изменения...

  feat(daemon): add smart commit tools ✨
  
  - Add suggest_commit() MCP tool for commit suggestions
  - Add auto_commit() for automatic commits
  - Integrate with git_diff and safety checks
  
  Files: src/daemon/tools.rs (+127 lines)
  
  Safety checks:
  ✅ No errors
  ✅ No secrets
  ⚠️  No tests (consider adding tests for new tools)
  
  [Approve] [Edit] [Cancel]

User: approve

gofer: ✅ Committed: a3f5b21
```

### Сценарий 2: Авто-коммит режим

```
User: Буду работать над фичей, автоматически коммить прогресс

gofer: enable auto-commit watch mode?

User: auto_commit --watch --trigger=on_save

gofer: 🔄 Watch mode enabled
  Commits will be created on file save
  Use 'commit_watch stop' to disable

[... работаешь 2 часа ...]

gofer: ✅ a3f5b21: feat: add analyzer module
gofer: ✅ b4c8d32: feat: add generator module  
gofer: ✅ e7f1a54: test: add tests for analyzer
gofer: ✅ 3a9d6b1: fix: handle edge case in generator

User: commit_watch stop

gofer: ⏸️  Watch stopped
  Created 4 commits during session
  
  Squash into single commit? [Yes] [No] [Let me review]

User: yes

gofer: ✅ Squashed into: feat(commit): implement smart commit system ✨
  Original commits archived in ORIG_HEAD
```

### Сценарий 3: Большое изменение

```
# Много файлов изменено
User: suggest_commit

gofer: 📝 Анализирую изменения...

  ⚠️  Large commit detected (15 files)
  
  Рекомендация: разбить на несколько коммитов по модулям?
  
  Предлагаемая разбивка:
  
  1. feat(analyzer): implement change analyzer (5 files)
  2. feat(generator): implement message generator (4 files)
  3. feat(safety): add safety checks (3 files)
  4. test: add comprehensive tests (3 files)
  
  [Create 4 commits] [Single commit anyway] [Cancel]

User: Create 4 commits

gofer: ✅ Creating 4 commits...
  a3f5b21: feat(analyzer): implement change analyzer
  b4c8d32: feat(generator): implement message generator
  e7f1a54: feat(safety): add safety checks
  3a9d6b1: test: add comprehensive tests
  
  Done! 4 logical commits created.
```

### Сценарий 4: Безопасность

```
User: auto_commit --force

gofer: 🔍 Safety check...
  
  ❌ Cannot commit: safety violations
  
  Errors:
  - .env file in staging area (contains API keys)
  - 2 compilation errors in src/main.rs
  
  Warnings:
  - 8 TODO comments in staged files
  
  Рекомендация:
  1. Remove .env from staging: git reset HEAD .env
  2. Fix compilation errors: cargo check
  3. Resolve TODOs or create issues
  
  Cannot force commit with errors (only warnings can be ignored).

User: git reset HEAD .env

User: auto_commit

gofer: 🔍 Safety check...
  
  ⚠️  Warnings:
  - 2 compilation errors remain
  - 8 TODO comments
  
  Continue anyway? [Yes] [No] [Fix first]

User: Fix first

gofer: Good choice! Run 'cargo check' to see errors.
```

---

## 🎨 Emoji Mapping

**Conventional Commits → Emoji:**

| Type | Emoji | Example |
|------|-------|---------|
| feat | ✨ | feat: add new feature ✨ |
| fix | 🐛 | fix: resolve bug ✨ |
| refactor | ♻️ | refactor: simplify logic ♻️ |
| perf | ⚡ | perf: optimize algorithm ⚡ |
| docs | 📝 | docs: update README 📝 |
| test | ✅ | test: add unit tests ✅ |
| chore | 🔧 | chore: update dependencies 🔧 |
| style | 💄 | style: format code 💄 |
| ci | 👷 | ci: update workflow 👷 |
| build | 📦 | build: update build config 📦 |

**Опционально, можно отключить через `include_emoji: false`**

---

## 🔐 Безопасность

### Что проверяется ВСЕГДА (errors):

1. ❌ **Compilation errors** - не коммитим сломанный код
2. ❌ **Secret files** - .env, *.key, credentials.json
3. ❌ **Git state** - detached HEAD, merge/rebase in progress
4. ❌ **Secret content** - API keys, passwords в коде

### Что проверяется с предупреждением (warnings):

1. ⚠️ **Large commits** - >10 файлов (можно force)
2. ⚠️ **No tests** - новый код без тестов
3. ⚠️ **TODO comments** - uncommitted TODOs
4. ⚠️ **Long subject** - >72 символа

### Force flag:

```
auto_commit --force
```
- **Игнорирует warnings** (large commits, no tests)
- **НЕ игнорирует errors** (secrets, broken code)

---

## 🚀 Фазы реализации

### Phase 1: MVP (1-2 дня) ✅

**Цель:** Базовый suggest_commit работает

- [x] Analyzer: parse git diff, classify change type
- [x] Generator: simple message generation
- [x] Safety: basic checks (secrets, compilation)
- [x] MCP tool: suggest_commit

**Результат:** Можно получить предложение коммита

### Phase 2: Auto-commit (2-3 дня)

**Цель:** Автоматический коммит с подтверждением

- [ ] Implement auto_commit tool
- [ ] Git integration: add + commit
- [ ] Safety enforcement
- [ ] Undo mechanism

**Результат:** Можно коммитить одной командой

### Phase 3: Smart generation (1 неделя)

**Цель:** Умная генерация на основе контекста

- [ ] Learn from git history (detect conventions)
- [ ] Semantic analysis (changed symbols, not just files)
- [ ] Body generation (detailed bullets)
- [ ] Breaking changes detection
- [ ] Related issues detection (через keywords в коде)

**Результат:** Сообщения как у опытного разработчика

### Phase 4: Watch mode (1 неделя)

**Цель:** Фоновые авто-коммиты

- [ ] File watcher integration
- [ ] Trigger system (on_save, on_test_pass, periodic)
- [ ] Batch management
- [ ] Smart squashing

**Результат:** Работаешь - gofer коммитит за тебя

### Phase 5: Advanced (2 недели)

**Цель:** Pro features

- [ ] Multi-commit splitting (large changes → logical commits)
- [ ] Co-author detection (pair programming)
- [ ] Issue tracker integration (Closes #123)
- [ ] LLM-powered generation (через Ollama)
- [ ] Commit templates
- [ ] Git hooks integration

---

## 🎯 Success Metrics

### Quantitative:
- **Time saved:** 2-5 минут на коммит → 10 секунд
- **Commit quality:** 80%+ информативных сообщений
- **Adoption:** 90%+ коммитов через smart_commit
- **Safety:** 0 коммитов с секретами

### Qualitative:
- Коммиты читаемы и понятны через месяцы
- Новые участники понимают историю
- Не тратишь ментальную энергию на формулировки
- Git log как документация

---

## 🤔 Открытые вопросы

### 1. LLM для генерации?

**Опции:**
- A) Rule-based (быстро, детерминировано, бесплатно)
- B) Local LLM через Ollama (умнее, медленнее, приватно)
- C) Cloud API (OpenAI/Anthropic) (самое умное, стоит денег, latency)

**Рекомендация:** Start с rule-based, добавить LLM как optional enhancement

### 2. Язык коммитов?

**Опции:**
- A) English (universal)
- B) Russian (удобнее для русскоязычных)
- C) Auto-detect from git history

**Рекомендация:** Auto-detect, fallback на English

### 3. Conventional Commits enforcement?

**Опции:**
- A) Всегда использовать (строго)
- B) Определять из истории (адаптивно)
- C) Опция в конфиге

**Рекомендация:** Auto-detect + можно override в конфиге

### 4. Squashing strategy?

**Опции:**
- A) Никогда не squash (preserve history)
- B) Всегда предлагать squash (clean history)
- C) Smart: squash WIP/micro-commits, keep semantic

**Рекомендация:** Smart squashing

---

## 📚 Референсы

**Inspiration:**
- [Conventional Commits](https://www.conventionalcommits.org/)
- [Gitmoji](https://gitmoji.dev/)
- [Angular Commit Guidelines](https://github.com/angular/angular/blob/main/CONTRIBUTING.md#commit)

**Similar tools:**
- `git-cliff` - changelog generator
- `commitizen` - interactive commit tool
- `commitlint` - lint commit messages

**gofer advantages:**
- Context-aware (знает весь проект)
- Semantic analysis (понимает код, не только diff)
- Integrated (часть ecosystem, не standalone tool)
- Adaptive (учится от истории проекта)

---

## 💬 Feedback & Iteration

**Вопросы для обсуждения:**

1. Какой режим по умолчанию? (suggest vs auto)
2. Нужен ли watch mode в MVP?
3. LLM-генерация - must-have или nice-to-have?
4. Какие ещё safety checks добавить?

**Предложения по улучшению:**
- Открывайте issues с тегом `feature:smart-commit`
- Пишите примеры use cases которые должны работать
- Предлагайте улучшения для commit message качества

---

## ✅ Next Steps

**Immediate:**
1. Review этого design doc
2. Approve или iterate
3. Start implementation Phase 1

**После MVP:**
1. Dogfooding на gofer проекте
2. Collect feedback
3. Iterate на основе реального использования
4. Phase 2-5 по мере необходимости

---

**Готов начать implementation?** 🚀

Let's make commits great again! ✨
