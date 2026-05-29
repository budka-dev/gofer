# Feature: suggest_commit - Умная генерация коммитов

**ID:** PHASE0-007  
**Priority:** 🔥🔥 High  
**Effort:** 7 дней (1 неделя)  
**Status:** Not Started  
**Phase:** 0 (Quick Wins)  
**Related:** SMART_COMMIT_DESIGN.md (полный дизайн)

---

## 📋 Описание

MCP tool для автоматической генерации quality commit messages на основе анализа git diff. MVP версия из полного SMART_COMMIT_DESIGN - фокус на suggest mode (без auto/watch).

### Проблема

**Текущий workflow:**
```bash
# 1. Write code
$ vim src/auth.rs

# 2. git diff чтобы вспомнить что изменил
$ git diff

# 3. Думать что писать в коммите (2-3 минуты)
$ git commit -m "???"

# 4. Писать generic message
$ git commit -m "update auth"  # Неинформативно
```

**Проблемы:**
- Тратишь время на формулировку (2-5 мин)
- Забываешь детали изменений
- Коммиты generic: "fix", "update", "wip"
- Нет consistency в стиле
- Пропускаешь важные детали

### Решение

```bash
# AI generates commit based on changes
$ qoder "suggest commit message"

gofer: 📝 Suggested commit:

  feat(auth): add JWT token verification with expiration check
  
  - Implement verify_token() function with RSA signature validation
  - Add expiration time check with 1-hour tolerance
  - Return structured Claims with user_id and permissions
  - Add comprehensive error handling for invalid tokens
  
  Files changed: src/auth/verify.rs (+87 lines)
  
  [Approve] [Edit] [Cancel]
```

**Benefits:**
- 0 time thinking
- Detailed, informative messages
- Consistent style (Conventional Commits)
- Captures all important changes
- Safe (shows preview before commit)

---

## 🎯 Goals & Non-Goals

### Goals (MVP)
- ✅ Analyze git diff and generate commit message
- ✅ Follow Conventional Commits format
- ✅ Include emoji (configurable)
- ✅ Detect change type (feat/fix/refactor/docs/etc)
- ✅ Safety checks (secrets, compilation errors)
- ✅ Show preview before committing

### Non-Goals (Future phases)
- ❌ Auto-commit without confirmation (Phase 2)
- ❌ Watch mode (Phase 4)
- ❌ LLM-powered generation (use rule-based for MVP)
- ❌ Multi-commit splitting (Phase 5)
- ❌ Learn from git history (nice-to-have)

---

## 🏗️ Архитектура

### Components

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│       suggest_commit()                  │
└────────────────┬────────────────────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│ Change   │ │Msg   │ │  Safety   │
│ Analyzer │ │Gen.  │ │  Checker  │
└──────────┘ └──────┘ └───────────┘
```

### Pipeline

```
1. Get git diff → 2. Analyze changes → 3. Generate message → 4. Safety check
   ↓               ↓                    ↓                    ↓
git diff        Parse diff           Rule-based          Check:
--staged        Classify type        generator           - Secrets
+ unstaged      Detect scope                             - Compilation
                Count stats                              - Large commit
                                                          
                                     ↓
                5. Return suggestion to user (no commit yet)
```

---

## 🔧 API Specification

### MCP Tool Definition

```json
{
  "name": "suggest_commit",
  "description": "Generate commit message suggestion based on git changes",
  "inputSchema": {
    "type": "object",
    "properties": {
      "files": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Specific files to include (null = all staged/modified)"
      },
      "style": {
        "type": "string",
        "enum": ["conventional", "simple", "detailed"],
        "default": "conventional",
        "description": "Commit message style"
      },
      "include_emoji": {
        "type": "boolean",
        "default": true,
        "description": "Add emoji to subject line"
      },
      "max_subject_length": {
        "type": "integer",
        "default": 72,
        "description": "Maximum subject line length"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct SuggestCommitResponse {
    pub suggested_message: CommitMessage,
    pub files: Vec<FileChange>,
    pub analysis: ChangeAnalysis,
    pub safety_check: SafetyReport,
    pub can_commit: bool,
}

#[derive(Serialize)]
pub struct CommitMessage {
    pub subject: String,
    pub body: Option<String>,
    pub full_message: String,  // subject + \n\n + body
}

#[derive(Serialize)]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,  // "added", "modified", "deleted", "renamed"
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Serialize)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Serialize)]
pub struct ChangeAnalysis {
    pub change_type: ChangeType,
    pub scope: String,
    pub complexity: Complexity,
    pub summary: String,
}

#[derive(Serialize)]
pub enum ChangeType {
    Feature,      // feat:
    Fix,          // fix:
    Refactor,     // refactor:
    Docs,         // docs:
    Test,         // test:
    Chore,        // chore:
    Perf,         // perf:
    Style,        // style:
    Build,        // build:
    CI,           // ci:
}

#[derive(Serialize)]
pub enum Complexity {
    Simple,    // 1-3 files, < 100 lines
    Medium,    // 4-10 files, 100-500 lines
    Large,     // > 10 files or > 500 lines
}

#[derive(Serialize)]
pub struct SafetyReport {
    pub can_commit: bool,
    pub errors: Vec<SafetyError>,
    pub warnings: Vec<SafetyWarning>,
}

#[derive(Serialize)]
pub struct SafetyError {
    pub severity: Severity,
    pub message: String,
    pub blocking: bool,
}

#[derive(Serialize)]
pub struct SafetyWarning {
    pub message: String,
    pub recommendation: String,
}

#[derive(Serialize)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}
```

### Example Response

```json
{
  "suggested_message": {
    "subject": "feat(auth): add JWT token verification ✨",
    "body": "- Implement verify_token() with RSA signature validation\n- Add expiration check with 1-hour tolerance\n- Return structured Claims with user_id and permissions\n- Add error handling for invalid/expired tokens",
    "full_message": "feat(auth): add JWT token verification ✨\n\n- Implement verify_token() with RSA signature validation\n- Add expiration check with 1-hour tolerance\n- Return structured Claims with user_id and permissions\n- Add error handling for invalid/expired tokens"
  },
  "files": [
    {
      "path": "src/auth/verify.rs",
      "status": "Added",
      "additions": 87,
      "deletions": 0
    },
    {
      "path": "src/auth/mod.rs",
      "status": "Modified",
      "additions": 2,
      "deletions": 0
    }
  ],
  "analysis": {
    "change_type": "Feature",
    "scope": "auth",
    "complexity": "Simple",
    "summary": "New JWT verification functionality added to auth module"
  },
  "safety_check": {
    "can_commit": true,
    "errors": [],
    "warnings": [
      {
        "message": "No tests found for new code",
        "recommendation": "Consider adding tests for verify_token()"
      }
    ]
  },
  "can_commit": true
}
```

---

## 💻 Implementation Details

### 1. Change Analyzer

```rust
// src/commit/analyzer.rs

use git2::{Repository, Diff, DiffOptions};

pub struct ChangeAnalyzer {
    repo: Repository,
}

impl ChangeAnalyzer {
    pub fn new(repo_path: &Path) -> Result<Self> {
        let repo = Repository::open(repo_path)?;
        Ok(Self { repo })
    }
    
    pub fn analyze(&self, files: Option<Vec<String>>) -> Result<ChangeAnalysis> {
        // Get diff
        let diff = self.get_diff(files)?;
        
        // Parse changes
        let file_changes = self.parse_file_changes(&diff)?;
        
        // Detect change type
        let change_type = self.detect_change_type(&file_changes);
        
        // Detect scope
        let scope = self.detect_scope(&file_changes);
        
        // Calculate complexity
        let complexity = self.calculate_complexity(&file_changes);
        
        // Generate summary
        let summary = self.generate_summary(&file_changes, &change_type);
        
        Ok(ChangeAnalysis {
            change_type,
            scope,
            complexity,
            summary,
            file_changes,
        })
    }
    
    fn get_diff(&self, files: Option<Vec<String>>) -> Result<Diff> {
        let head = self.repo.head()?.peel_to_tree()?;
        let mut opts = DiffOptions::new();
        
        // Filter by files if specified
        if let Some(paths) = files {
            for path in paths {
                opts.pathspec(path);
            }
        }
        
        // Get staged + unstaged changes
        let diff = self.repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?;
        
        Ok(diff)
    }
    
    fn parse_file_changes(&self, diff: &Diff) -> Result<Vec<FileChange>> {
        let mut changes = Vec::new();
        
        diff.foreach(
            &mut |delta, _progress| {
                let path = delta.new_file().path()
                    .and_then(|p| p.to_str())
                    .unwrap_or("");
                
                let status = match delta.status() {
                    git2::Delta::Added => FileStatus::Added,
                    git2::Delta::Modified => FileStatus::Modified,
                    git2::Delta::Deleted => FileStatus::Deleted,
                    git2::Delta::Renamed => {
                        let from = delta.old_file().path()
                            .and_then(|p| p.to_str())
                            .unwrap_or("")
                            .to_string();
                        FileStatus::Renamed { from }
                    }
                    _ => return true,
                };
                
                changes.push(FileChange {
                    path: path.to_string(),
                    status,
                    additions: 0,  // Will be filled by stats
                    deletions: 0,
                });
                
                true
            },
            None,
            None,
            None,
        )?;
        
        // Get stats
        let stats = diff.stats()?;
        
        // TODO: Map stats to file changes
        
        Ok(changes)
    }
    
    fn detect_change_type(&self, files: &[FileChange]) -> ChangeType {
        // Heuristics for change type detection
        
        // Only docs changed → Docs
        if files.iter().all(|f| self.is_doc_file(&f.path)) {
            return ChangeType::Docs;
        }
        
        // Only tests changed → Test
        if files.iter().all(|f| self.is_test_file(&f.path)) {
            return ChangeType::Test;
        }
        
        // New files (not tests/docs) → Feature
        if files.iter().any(|f| matches!(f.status, FileStatus::Added)) &&
           !files.iter().all(|f| self.is_test_file(&f.path))
        {
            return ChangeType::Feature;
        }
        
        // Renamed/moved → Refactor
        if files.iter().any(|f| matches!(f.status, FileStatus::Renamed { .. })) {
            return ChangeType::Refactor;
        }
        
        // TODO: More sophisticated detection
        // - Look at commit message patterns in history
        // - Analyze function changes (new vs modified)
        // - Check for bug-related keywords in diff
        
        // Default: infer from file changes
        ChangeType::Feature
    }
    
    fn detect_scope(&self, files: &[FileChange]) -> String {
        // Extract common prefix/module
        
        if files.is_empty() {
            return "general".to_string();
        }
        
        // Find common path component
        let paths: Vec<&str> = files.iter()
            .map(|f| f.path.as_str())
            .collect();
        
        let common = self.find_common_prefix(&paths);
        
        // Extract module name
        if common.contains('/') {
            common.split('/').nth(1)
                .or_else(|| common.split('/').next())
                .unwrap_or("general")
                .to_string()
        } else {
            "general".to_string()
        }
    }
    
    fn find_common_prefix(&self, paths: &[&str]) -> String {
        if paths.is_empty() {
            return String::new();
        }
        
        let first = paths[0];
        let mut prefix = String::new();
        
        'outer: for (i, c) in first.chars().enumerate() {
            for path in paths.iter().skip(1) {
                if path.chars().nth(i) != Some(c) {
                    break 'outer;
                }
            }
            prefix.push(c);
        }
        
        prefix
    }
    
    fn calculate_complexity(&self, files: &[FileChange]) -> Complexity {
        let total_files = files.len();
        let total_lines: usize = files.iter()
            .map(|f| f.additions + f.deletions)
            .sum();
        
        if total_files <= 3 && total_lines < 100 {
            Complexity::Simple
        } else if total_files <= 10 && total_lines < 500 {
            Complexity::Medium
        } else {
            Complexity::Large
        }
    }
    
    fn generate_summary(
        &self,
        files: &[FileChange],
        change_type: &ChangeType,
    ) -> String {
        let action = match change_type {
            ChangeType::Feature => "New functionality added",
            ChangeType::Fix => "Bug fix",
            ChangeType::Refactor => "Code refactoring",
            ChangeType::Docs => "Documentation update",
            ChangeType::Test => "Tests added/updated",
            ChangeType::Chore => "Maintenance task",
            ChangeType::Perf => "Performance improvement",
            ChangeType::Style => "Code style changes",
            ChangeType::Build => "Build configuration update",
            ChangeType::CI => "CI/CD update",
        };
        
        format!("{} in {} file(s)", action, files.len())
    }
    
    fn is_doc_file(&self, path: &str) -> bool {
        path.ends_with(".md") ||
        path.ends_with(".txt") ||
        path.contains("docs/") ||
        path.contains("README")
    }
    
    fn is_test_file(&self, path: &str) -> bool {
        path.contains("test") ||
        path.contains("spec") ||
        path.ends_with("_test.rs") ||
        path.ends_with(".test.ts")
    }
}
```

### 2. Message Generator

```rust
// src/commit/generator.rs

pub struct MessageGenerator {
    style: CommitStyle,
    emoji_enabled: bool,
    max_subject_length: usize,
}

impl MessageGenerator {
    pub fn new(
        style: CommitStyle,
        emoji_enabled: bool,
        max_subject_length: usize,
    ) -> Self {
        Self {
            style,
            emoji_enabled,
            max_subject_length,
        }
    }
    
    pub fn generate(&self, analysis: &ChangeAnalysis) -> CommitMessage {
        let subject = self.generate_subject(analysis);
        let body = self.generate_body(analysis);
        
        let full_message = if let Some(ref body_text) = body {
            format!("{}\n\n{}", subject, body_text)
        } else {
            subject.clone()
        };
        
        CommitMessage {
            subject,
            body,
            full_message,
        }
    }
    
    fn generate_subject(&self, analysis: &ChangeAnalysis) -> String {
        // Build subject line
        let mut parts = Vec::new();
        
        // 1. Type + scope (for conventional style)
        if matches!(self.style, CommitStyle::Conventional) {
            let type_str = self.change_type_str(&analysis.change_type);
            let scope = &analysis.scope;
            parts.push(format!("{}({})", type_str, scope));
        }
        
        // 2. Description
        let description = self.generate_description(analysis);
        parts.push(description);
        
        // 3. Emoji (if enabled)
        if self.emoji_enabled {
            let emoji = self.emoji_for_type(&analysis.change_type);
            parts.push(emoji);
        }
        
        // Combine and truncate
        let mut subject = parts.join(": ");
        if subject.len() > self.max_subject_length {
            subject.truncate(self.max_subject_length - 3);
            subject.push_str("...");
        }
        
        subject
    }
    
    fn generate_description(&self, analysis: &ChangeAnalysis) -> String {
        // Generate concise description based on changes
        
        // TODO: More sophisticated description generation
        // For MVP, use simple summary
        
        analysis.summary.clone()
    }
    
    fn generate_body(&self, analysis: &ChangeAnalysis) -> Option<String> {
        // Only generate body for detailed style or complex changes
        if !matches!(self.style, CommitStyle::Detailed) &&
           matches!(analysis.complexity, Complexity::Simple)
        {
            return None;
        }
        
        let mut lines = Vec::new();
        
        // Add bullet points for each file change
        for file in &analysis.file_changes {
            let change_desc = self.describe_file_change(file);
            lines.push(format!("- {}", change_desc));
        }
        
        Some(lines.join("\n"))
    }
    
    fn describe_file_change(&self, file: &FileChange) -> String {
        match &file.status {
            FileStatus::Added => {
                format!("Add {} (+{} lines)", file.path, file.additions)
            }
            FileStatus::Modified => {
                format!("Update {} (+{} -{} lines)", 
                    file.path, file.additions, file.deletions)
            }
            FileStatus::Deleted => {
                format!("Remove {}", file.path)
            }
            FileStatus::Renamed { from } => {
                format!("Rename {} → {}", from, file.path)
            }
        }
    }
    
    fn change_type_str(&self, change_type: &ChangeType) -> &str {
        match change_type {
            ChangeType::Feature => "feat",
            ChangeType::Fix => "fix",
            ChangeType::Refactor => "refactor",
            ChangeType::Docs => "docs",
            ChangeType::Test => "test",
            ChangeType::Chore => "chore",
            ChangeType::Perf => "perf",
            ChangeType::Style => "style",
            ChangeType::Build => "build",
            ChangeType::CI => "ci",
        }
    }
    
    fn emoji_for_type(&self, change_type: &ChangeType) -> String {
        match change_type {
            ChangeType::Feature => "✨",
            ChangeType::Fix => "🐛",
            ChangeType::Refactor => "♻️",
            ChangeType::Perf => "⚡",
            ChangeType::Docs => "📝",
            ChangeType::Test => "✅",
            ChangeType::Chore => "🔧",
            ChangeType::Style => "💄",
            ChangeType::Build => "📦",
            ChangeType::CI => "👷",
        }.to_string()
    }
}

pub enum CommitStyle {
    Conventional,  // feat(scope): subject
    Simple,        // Subject only
    Detailed,      // Subject + body
}
```

### 3. Safety Checker

```rust
// src/commit/safety.rs

pub struct SafetyChecker {
    repo_path: PathBuf,
}

impl SafetyChecker {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
    
    pub async fn check(&self, analysis: &ChangeAnalysis) -> Result<SafetyReport> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // 1. Check for secrets
        if let Some(secret_err) = self.check_secrets(&analysis.file_changes).await {
            errors.push(secret_err);
        }
        
        // 2. Check compilation (if applicable)
        if let Some(compile_err) = self.check_compilation().await {
            errors.push(compile_err);
        }
        
        // 3. Check for large commits
        if let Some(warning) = self.check_large_commit(analysis) {
            warnings.push(warning);
        }
        
        // 4. Check for missing tests
        if let Some(warning) = self.check_missing_tests(analysis) {
            warnings.push(warning);
        }
        
        let can_commit = errors.is_empty();
        
        Ok(SafetyReport {
            can_commit,
            errors,
            warnings,
        })
    }
    
    async fn check_secrets(&self, files: &[FileChange]) -> Option<SafetyError> {
        let secret_patterns = [
            ".env",
            "credentials.json",
            "*.key",
            "*.pem",
            "*_rsa",
        ];
        
        let secret_files: Vec<_> = files.iter()
            .filter(|f| {
                secret_patterns.iter().any(|pattern| {
                    // Simple pattern matching
                    if pattern.starts_with('*') {
                        f.path.ends_with(&pattern[1..])
                    } else {
                        f.path.contains(pattern)
                    }
                })
            })
            .collect();
        
        if !secret_files.is_empty() {
            Some(SafetyError {
                severity: Severity::Critical,
                message: format!(
                    "Potential secret files detected: {}",
                    secret_files.iter()
                        .map(|f| f.path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                blocking: true,
            })
        } else {
            None
        }
    }
    
    async fn check_compilation(&self) -> Option<SafetyError> {
        // Check if this is a Rust project
        if !self.repo_path.join("Cargo.toml").exists() {
            return None;
        }
        
        // Run cargo check
        let output = tokio::process::Command::new("cargo")
            .arg("check")
            .arg("--message-format=short")
            .current_dir(&self.repo_path)
            .output()
            .await;
        
        match output {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Some(SafetyError {
                        severity: Severity::High,
                        message: format!("Compilation errors detected:\n{}", stderr),
                        blocking: true,
                    })
                } else {
                    None
                }
            }
            Err(_) => None,  // cargo not available or other error
        }
    }
    
    fn check_large_commit(&self, analysis: &ChangeAnalysis) -> Option<SafetyWarning> {
        if matches!(analysis.complexity, Complexity::Large) {
            Some(SafetyWarning {
                message: "Large commit detected (>10 files or >500 lines)".to_string(),
                recommendation: "Consider splitting into smaller, logical commits".to_string(),
            })
        } else {
            None
        }
    }
    
    fn check_missing_tests(&self, analysis: &ChangeAnalysis) -> Option<SafetyWarning> {
        // Check if any new code added without tests
        let has_new_code = analysis.file_changes.iter()
            .any(|f| {
                matches!(f.status, FileStatus::Added | FileStatus::Modified) &&
                !self.is_test_file(&f.path)
            });
        
        let has_test_changes = analysis.file_changes.iter()
            .any(|f| self.is_test_file(&f.path));
        
        if has_new_code && !has_test_changes {
            Some(SafetyWarning {
                message: "No tests found for new/modified code".to_string(),
                recommendation: "Consider adding tests to verify functionality".to_string(),
            })
        } else {
            None
        }
    }
    
    fn is_test_file(&self, path: &str) -> bool {
        path.contains("test") || path.contains("spec")
    }
}
```

### 4. MCP Tool Handler

```rust
// src/daemon/tools/suggest_commit.rs

pub async fn handle_suggest_commit(
    args: &Map<String, Value>,
    workspace_root: &Path,
) -> Result<Value> {
    // Parse arguments
    let files = args.get("files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
    
    let style = args.get("style")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "simple" => Some(CommitStyle::Simple),
            "detailed" => Some(CommitStyle::Detailed),
            _ => Some(CommitStyle::Conventional),
        })
        .unwrap_or(CommitStyle::Conventional);
    
    let include_emoji = args.get("include_emoji")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let max_subject_length = args.get("max_subject_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(72) as usize;
    
    // Analyze changes
    let analyzer = ChangeAnalyzer::new(workspace_root)?;
    let analysis = analyzer.analyze(files)?;
    
    // Generate message
    let generator = MessageGenerator::new(style, include_emoji, max_subject_length);
    let suggested_message = generator.generate(&analysis);
    
    // Safety check
    let checker = SafetyChecker::new(workspace_root.to_path_buf());
    let safety_check = checker.check(&analysis).await?;
    
    let response = SuggestCommitResponse {
        suggested_message,
        files: analysis.file_changes,
        analysis: ChangeAnalysis {
            change_type: analysis.change_type,
            scope: analysis.scope,
            complexity: analysis.complexity,
            summary: analysis.summary,
        },
        safety_check,
        can_commit: safety_check.can_commit,
    };
    
    Ok(serde_json::to_value(response)?)
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_detect_change_type() {
    let analyzer = ChangeAnalyzer::new(Path::new("test_repo")).unwrap();
    
    // Feature: new files
    let files = vec![
        FileChange {
            path: "src/new_feature.rs".into(),
            status: FileStatus::Added,
            additions: 100,
            deletions: 0,
        }
    ];
    let change_type = analyzer.detect_change_type(&files);
    assert!(matches!(change_type, ChangeType::Feature));
    
    // Docs: only markdown
    let files = vec![
        FileChange {
            path: "docs/guide.md".into(),
            status: FileStatus::Modified,
            additions: 50,
            deletions: 10,
        }
    ];
    let change_type = analyzer.detect_change_type(&files);
    assert!(matches!(change_type, ChangeType::Docs));
}

#[tokio::test]
async fn test_message_generation() {
    let generator = MessageGenerator::new(
        CommitStyle::Conventional,
        true,
        72,
    );
    
    let analysis = ChangeAnalysis {
        change_type: ChangeType::Feature,
        scope: "auth".to_string(),
        complexity: Complexity::Simple,
        summary: "Add JWT verification".to_string(),
        file_changes: vec![],
    };
    
    let message = generator.generate(&analysis);
    
    assert!(message.subject.starts_with("feat(auth):"));
    assert!(message.subject.contains("✨"));
    assert!(message.subject.len() <= 72);
}

#[tokio::test]
async fn test_safety_check_secrets() {
    let checker = SafetyChecker::new(PathBuf::from("test_repo"));
    
    let files = vec![
        FileChange {
            path: ".env".into(),
            status: FileStatus::Modified,
            additions: 5,
            deletions: 0,
        }
    ];
    
    let analysis = ChangeAnalysis {
        change_type: ChangeType::Chore,
        scope: "config".to_string(),
        complexity: Complexity::Simple,
        summary: "Update config".to_string(),
        file_changes: files,
    };
    
    let report = checker.check(&analysis).await.unwrap();
    
    assert!(!report.can_commit);
    assert!(!report.errors.is_empty());
}
```

---

## 📈 Success Metrics

- ✅ 80%+ of generated messages are informative (vs generic)
- ✅ Follow Conventional Commits format
- ✅ Safety checks catch secrets 100% of time
- ⏱️ Response time < 2 seconds
- ✅ No false positive blocks (can commit valid changes)

---

## 📚 Usage Examples

```typescript
// Basic usage
const suggestion = await gofer.suggest_commit();

console.log(suggestion.suggested_message.full_message);

if (suggestion.can_commit) {
  // User approves, then use git directly
  exec(`git commit -m "${suggestion.suggested_message.full_message}"`);
}

// Custom style
const suggestion = await gofer.suggest_commit({
  style: "simple",
  include_emoji: false
});
```

---

## ✅ Acceptance Criteria

- [ ] Analyzes git diff correctly
- [ ] Detects change type with 70%+ accuracy
- [ ] Generates Conventional Commits format
- [ ] Emoji mapping works
- [ ] Safety checks block secrets
- [ ] Safety checks detect compilation errors (Rust)
- [ ] Large commit warning works
- [ ] All unit tests pass
- [ ] Documentation complete

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Effort:** 7 days (MVP only, no auto/watch modes)
