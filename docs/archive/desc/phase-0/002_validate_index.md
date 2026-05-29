# Feature: validate_index - Поиск проблем в индексе

**ID:** PHASE0-002  
**Priority:** 🔥🔥🔥 Critical  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)  
**Depends On:** 001_get_index_status

---

## 📋 Описание

Инструмент для глубокой валидации индекса gofer MCP с поиском gaps, inconsistencies и corrupted data. Не просто показывает статус (это делает `get_index_status`), а активно ищет проблемы и предлагает решения.

### Проблема

Индекс может иметь проблемы, которые не видны в общей статистике:
- **Missing trait impls** - трейты реализованы в коде, но не в индексе
- **Broken references** - символ ссылается на несуществующий файл
- **Outdated embeddings** - файл изменился, но embeddings старые
- **Orphaned symbols** - символы без файла (файл удален, но символы остались)
- **Corrupted vectors** - embeddings с неправильной dimension
- **Duplicate entries** - один символ проиндексирован несколько раз

Эти проблемы приводят к:
- Неполным результатам поиска
- Неправильным callers/callees
- Broken links в результатах
- Wasted storage space

### Решение

MCP tool `validate_index` который проводит comprehensive audit индекса и возвращает детальный отчет с actionable recommendations.

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Найти все gaps и inconsistencies в индексе
- ✅ Предложить concrete fixes для каждой проблемы
- ✅ Поддерживать incremental validation (только измененные файлы)
- ✅ Выдавать machine-readable отчет для автоматизации

### Non-Goals
- ❌ Не исправляет проблемы автоматически (это делает `force_reindex`)
- ❌ Не валидирует correctness кода (это делает компилятор)
- ❌ Не проверяет качество summaries (субъективно)

---

## 🏗️ Архитектура

### Validation Pipeline

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│       validate_index()                  │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │   Validation    │
        │     Runner      │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬───────────┬───────────┐
     │           │           │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐ ┌──▼───┐ ┌─────▼────┐
│ Files    │ │Symbol│ │References │ │Vector│ │ Summary  │
│Validator │ │Valid.│ │ Validator │ │Valid.│ │Validator │
└──────────┘ └──────┘ └───────────┘ └──────┘ └──────────┘
```

### Validators

Each validator is independent and returns a list of issues.

```rust
pub trait Validator {
    async fn validate(&self, scope: ValidationScope) -> Result<Vec<ValidationIssue>>;
    fn name(&self) -> &str;
    fn estimated_duration(&self) -> Duration;
}
```

---

## 📊 Data Model

### Validation Issue Schema

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub id: String,              // Unique issue ID
    pub severity: IssueSeverity,
    pub category: IssueCategory,
    pub message: String,
    pub details: IssueDetails,
    pub affected_items: Vec<AffectedItem>,
    pub recommendation: Recommendation,
    pub auto_fixable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,  // Blocks functionality (e.g., broken references)
    High,      // Causes incorrect results (e.g., missing symbols)
    Medium,    // Causes incomplete results (e.g., missing embeddings)
    Low,       // Minor optimization opportunity (e.g., duplicates)
    Info,      // Informational only
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueCategory {
    MissingData,        // Expected data not found
    InconsistentData,   // Data conflicts
    CorruptedData,      // Data is invalid
    OutdatedData,       // Data is stale
    DuplicateData,      // Redundant data
    OrphanedData,       // Data without parent
    BrokenReferences,   // References to non-existent items
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDetails {
    pub description: String,
    pub impact: String,
    pub root_cause: Option<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedItem {
    pub item_type: String,  // "file", "symbol", "chunk", etc.
    pub item_id: String,
    pub item_path: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub action: RecommendedAction,
    pub command: Option<String>,  // Shell command to fix
    pub estimated_time: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecommendedAction {
    ReindexFile { path: String },
    ReindexFiles { paths: Vec<String> },
    ReindexSymbols { file: String },
    RegenerateEmbeddings { chunk_ids: Vec<i64> },
    DeleteOrphanedData { ids: Vec<i64> },
    RebuildIndex,  // Nuclear option
    Manual { instructions: String },
}
```

---

## 🔧 API Specification

### MCP Tool Definition

```json
{
  "name": "validate_index",
  "description": "Validate gofer index integrity and find gaps, inconsistencies, corrupted data",
  "inputSchema": {
    "type": "object",
    "properties": {
      "scope": {
        "type": "string",
        "enum": ["full", "incremental", "files", "symbols", "embeddings"],
        "default": "full",
        "description": "Validation scope: full check or specific components"
      },
      "files": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Specific files to validate (only with scope=files)"
      },
      "fix_auto_fixable": {
        "type": "boolean",
        "default": false,
        "description": "Automatically fix issues that are safe to auto-fix"
      },
      "max_issues": {
        "type": "integer",
        "default": 100,
        "description": "Maximum issues to return (0 = unlimited)"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct ValidationReport {
    pub validation_id: String,
    pub timestamp: DateTime<Utc>,
    pub scope: ValidationScope,
    pub duration_seconds: u64,
    
    // Summary
    pub summary: ValidationSummary,
    
    // Issues by severity
    pub issues: Vec<ValidationIssue>,
    pub issues_by_severity: HashMap<IssueSeverity, usize>,
    pub issues_by_category: HashMap<IssueCategory, usize>,
    
    // Auto-fix results (if requested)
    pub auto_fix_results: Option<AutoFixResults>,
    
    // Recommendations
    pub overall_recommendation: String,
    pub estimated_fix_time: Duration,
}

#[derive(Serialize)]
pub struct ValidationSummary {
    pub total_issues: usize,
    pub critical_issues: usize,
    pub auto_fixable_issues: usize,
    pub items_validated: ValidationCounts,
    pub health_score: f32,  // 0.0 - 100.0
}

#[derive(Serialize)]
pub struct ValidationCounts {
    pub files_checked: usize,
    pub symbols_checked: usize,
    pub references_checked: usize,
    pub chunks_checked: usize,
    pub vectors_checked: usize,
}

#[derive(Serialize)]
pub struct AutoFixResults {
    pub fixed_issues: usize,
    pub failed_fixes: usize,
    pub fixed_issue_ids: Vec<String>,
    pub errors: Vec<FixError>,
}

#[derive(Serialize)]
pub struct FixError {
    pub issue_id: String,
    pub error: String,
}
```

### Example Response

```json
{
  "validation_id": "val_2026021610300001",
  "timestamp": "2026-02-16T10:30:00Z",
  "scope": "full",
  "duration_seconds": 8,
  "summary": {
    "total_issues": 15,
    "critical_issues": 2,
    "auto_fixable_issues": 10,
    "items_validated": {
      "files_checked": 44,
      "symbols_checked": 1250,
      "references_checked": 3500,
      "chunks_checked": 597,
      "vectors_checked": 597
    },
    "health_score": 87.3
  },
  "issues": [
    {
      "id": "ISS-001",
      "severity": "Critical",
      "category": "BrokenReferences",
      "message": "3 symbols reference deleted files",
      "details": {
        "description": "Symbols exist in database but their source files are deleted",
        "impact": "Search returns broken links, get_callers fails",
        "root_cause": "Files deleted but symbols not cleaned up",
        "examples": [
          "Symbol 'old_function' in deleted file 'src/old.rs'",
          "Symbol 'deprecated_struct' in deleted file 'src/deprecated.rs'"
        ]
      },
      "affected_items": [
        {
          "item_type": "symbol",
          "item_id": "sym_123",
          "item_path": "src/old.rs",
          "line": 45
        },
        {
          "item_type": "symbol",
          "item_id": "sym_456",
          "item_path": "src/deprecated.rs",
          "line": 12
        }
      ],
      "recommendation": {
        "action": {
          "DeleteOrphanedData": {
            "ids": [123, 456]
          }
        },
        "command": "DELETE FROM symbols WHERE id IN (123, 456)",
        "estimated_time": { "secs": 1 }
      },
      "auto_fixable": true
    },
    {
      "id": "ISS-002",
      "severity": "High",
      "category": "MissingData",
      "message": "5 trait implementations not indexed",
      "details": {
        "description": "Trait impls exist in code but not found in symbol index",
        "impact": "find_trait_impls returns incomplete results",
        "root_cause": "Tree-sitter parser may have skipped impl blocks",
        "examples": [
          "impl Display for MyStruct in src/display.rs:10",
          "impl From<String> for MyType in src/convert.rs:25"
        ]
      },
      "affected_items": [
        {
          "item_type": "file",
          "item_id": "file_789",
          "item_path": "src/display.rs",
          "line": 10
        }
      ],
      "recommendation": {
        "action": {
          "ReindexFiles": {
            "paths": ["src/display.rs", "src/convert.rs"]
          }
        },
        "command": "force_reindex --files src/display.rs src/convert.rs",
        "estimated_time": { "secs": 2 }
      },
      "auto_fixable": false
    },
    {
      "id": "ISS-003",
      "severity": "Medium",
      "category": "OutdatedData",
      "message": "12 chunks have outdated embeddings",
      "details": {
        "description": "Source files modified but embeddings not regenerated",
        "impact": "Semantic search may return stale results",
        "root_cause": "File watcher missed change events or embedder failed",
        "examples": [
          "src/main.rs modified 2 hours ago, embeddings from yesterday"
        ]
      },
      "affected_items": [
        {
          "item_type": "chunk",
          "item_id": "chunk_321",
          "item_path": "src/main.rs",
          "line": 1
        }
      ],
      "recommendation": {
        "action": {
          "RegenerateEmbeddings": {
            "chunk_ids": [321, 322, 323]
          }
        },
        "estimated_time": { "secs": 5 }
      },
      "auto_fixable": true
    }
  ],
  "issues_by_severity": {
    "Critical": 2,
    "High": 3,
    "Medium": 7,
    "Low": 3,
    "Info": 0
  },
  "issues_by_category": {
    "BrokenReferences": 2,
    "MissingData": 5,
    "OutdatedData": 7,
    "DuplicateData": 1
  },
  "auto_fix_results": null,
  "overall_recommendation": "Run auto-fix to resolve 10 issues, then reindex 2 files manually",
  "estimated_fix_time": { "secs": 15 }
}
```

---

## 💻 Implementation Details

### Validation Runner

```rust
// src/indexer/validation/runner.rs

use std::time::Instant;
use uuid::Uuid;

pub struct ValidationRunner {
    sqlite: SqliteStorage,
    lance: LanceStorage,
    validators: Vec<Box<dyn Validator>>,
}

impl ValidationRunner {
    pub fn new(sqlite: SqliteStorage, lance: LanceStorage) -> Self {
        let validators: Vec<Box<dyn Validator>> = vec![
            Box::new(FilesValidator::new(sqlite.clone())),
            Box::new(SymbolsValidator::new(sqlite.clone())),
            Box::new(ReferencesValidator::new(sqlite.clone())),
            Box::new(VectorValidator::new(sqlite.clone(), lance.clone())),
            Box::new(SummaryValidator::new(sqlite.clone())),
        ];
        
        Self { sqlite, lance, validators }
    }
    
    pub async fn validate(
        &self,
        scope: ValidationScope,
        max_issues: Option<usize>,
    ) -> Result<ValidationReport> {
        let validation_id = format!("val_{}", Uuid::new_v4().simple());
        let start = Instant::now();
        
        // Run validators in parallel
        let mut all_issues = Vec::new();
        let mut validation_counts = ValidationCounts::default();
        
        let validators = self.filter_validators_by_scope(&scope);
        
        for validator in validators {
            info!("Running validator: {}", validator.name());
            
            let issues = validator.validate(scope.clone()).await?;
            let counts = validator.get_validation_counts().await?;
            
            all_issues.extend(issues);
            validation_counts.merge(counts);
        }
        
        // Sort by severity
        all_issues.sort_by(|a, b| a.severity.cmp(&b.severity));
        
        // Limit if requested
        if let Some(max) = max_issues {
            all_issues.truncate(max);
        }
        
        // Calculate summary
        let summary = self.calculate_summary(&all_issues, &validation_counts);
        
        // Group by severity and category
        let issues_by_severity = self.group_by_severity(&all_issues);
        let issues_by_category = self.group_by_category(&all_issues);
        
        // Overall recommendation
        let overall_recommendation = self.generate_overall_recommendation(&all_issues);
        let estimated_fix_time = self.estimate_fix_time(&all_issues);
        
        let duration = start.elapsed();
        
        Ok(ValidationReport {
            validation_id,
            timestamp: Utc::now(),
            scope,
            duration_seconds: duration.as_secs(),
            summary,
            issues: all_issues,
            issues_by_severity,
            issues_by_category,
            auto_fix_results: None,
            overall_recommendation,
            estimated_fix_time,
        })
    }
    
    pub async fn validate_and_fix(
        &self,
        scope: ValidationScope,
    ) -> Result<ValidationReport> {
        let mut report = self.validate(scope, None).await?;
        
        // Auto-fix fixable issues
        let auto_fixable: Vec<_> = report.issues.iter()
            .filter(|issue| issue.auto_fixable)
            .cloned()
            .collect();
        
        if !auto_fixable.is_empty() {
            let fix_results = self.auto_fix_issues(&auto_fixable).await?;
            report.auto_fix_results = Some(fix_results);
        }
        
        Ok(report)
    }
    
    async fn auto_fix_issues(
        &self,
        issues: &[ValidationIssue],
    ) -> Result<AutoFixResults> {
        let mut fixed_issues = 0;
        let mut failed_fixes = 0;
        let mut fixed_issue_ids = Vec::new();
        let mut errors = Vec::new();
        
        for issue in issues {
            match self.fix_issue(issue).await {
                Ok(_) => {
                    fixed_issues += 1;
                    fixed_issue_ids.push(issue.id.clone());
                }
                Err(e) => {
                    failed_fixes += 1;
                    errors.push(FixError {
                        issue_id: issue.id.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }
        
        Ok(AutoFixResults {
            fixed_issues,
            failed_fixes,
            fixed_issue_ids,
            errors,
        })
    }
    
    async fn fix_issue(&self, issue: &ValidationIssue) -> Result<()> {
        match &issue.recommendation.action {
            RecommendedAction::DeleteOrphanedData { ids } => {
                self.delete_orphaned_symbols(ids).await?;
            }
            RecommendedAction::RegenerateEmbeddings { chunk_ids } => {
                self.regenerate_embeddings(chunk_ids).await?;
            }
            _ => {
                return Err(anyhow!("Action not auto-fixable"));
            }
        }
        
        Ok(())
    }
    
    async fn delete_orphaned_symbols(&self, ids: &[i64]) -> Result<()> {
        for id in ids {
            sqlx::query!("DELETE FROM symbols WHERE id = ?", id)
                .execute(&self.sqlite.pool)
                .await?;
        }
        Ok(())
    }
    
    async fn regenerate_embeddings(&self, chunk_ids: &[i64]) -> Result<()> {
        // Mark chunks for re-embedding
        for id in chunk_ids {
            sqlx::query!(
                "UPDATE chunks SET needs_embedding = 1 WHERE id = ?",
                id
            )
            .execute(&self.sqlite.pool)
            .await?;
        }
        
        // Trigger background re-embedding
        // (actual embedding happens in background task)
        
        Ok(())
    }
    
    fn calculate_summary(
        &self,
        issues: &[ValidationIssue],
        counts: &ValidationCounts,
    ) -> ValidationSummary {
        let total_issues = issues.len();
        let critical_issues = issues.iter()
            .filter(|i| matches!(i.severity, IssueSeverity::Critical))
            .count();
        let auto_fixable_issues = issues.iter()
            .filter(|i| i.auto_fixable)
            .count();
        
        // Health score: 100 - (weighted sum of issues)
        let health_score = 100.0 - (
            critical_issues as f32 * 10.0 +
            issues.iter().filter(|i| matches!(i.severity, IssueSeverity::High)).count() as f32 * 5.0 +
            issues.iter().filter(|i| matches!(i.severity, IssueSeverity::Medium)).count() as f32 * 2.0 +
            issues.iter().filter(|i| matches!(i.severity, IssueSeverity::Low)).count() as f32 * 0.5
        ).max(0.0);
        
        ValidationSummary {
            total_issues,
            critical_issues,
            auto_fixable_issues,
            items_validated: counts.clone(),
            health_score,
        }
    }
    
    fn filter_validators_by_scope(&self, scope: &ValidationScope) -> Vec<&Box<dyn Validator>> {
        match scope {
            ValidationScope::Full => self.validators.iter().collect(),
            ValidationScope::Incremental => {
                // Only validators for recently changed files
                self.validators.iter()
                    .filter(|v| v.supports_incremental())
                    .collect()
            }
            ValidationScope::Files => {
                self.validators.iter()
                    .filter(|v| v.name() == "files")
                    .collect()
            }
            ValidationScope::Symbols => {
                self.validators.iter()
                    .filter(|v| v.name() == "symbols" || v.name() == "references")
                    .collect()
            }
            ValidationScope::Embeddings => {
                self.validators.iter()
                    .filter(|v| v.name() == "vectors")
                    .collect()
            }
        }
    }
    
    fn group_by_severity(&self, issues: &[ValidationIssue]) -> HashMap<IssueSeverity, usize> {
        let mut map = HashMap::new();
        for issue in issues {
            *map.entry(issue.severity.clone()).or_insert(0) += 1;
        }
        map
    }
    
    fn group_by_category(&self, issues: &[ValidationIssue]) -> HashMap<IssueCategory, usize> {
        let mut map = HashMap::new();
        for issue in issues {
            *map.entry(issue.category.clone()).or_insert(0) += 1;
        }
        map
    }
    
    fn generate_overall_recommendation(&self, issues: &[ValidationIssue]) -> String {
        let critical = issues.iter().filter(|i| matches!(i.severity, IssueSeverity::Critical)).count();
        let auto_fixable = issues.iter().filter(|i| i.auto_fixable).count();
        
        if critical > 0 {
            format!("CRITICAL: {} critical issues found. Fix immediately!", critical)
        } else if auto_fixable > 0 {
            format!("Run auto-fix to resolve {} issues automatically", auto_fixable)
        } else if !issues.is_empty() {
            "Some issues require manual intervention - see recommendations"
        } else {
            "Index is healthy, no issues found"
        }.to_string()
    }
    
    fn estimate_fix_time(&self, issues: &[ValidationIssue]) -> Duration {
        issues.iter()
            .map(|i| i.recommendation.estimated_time)
            .sum()
    }
}
```

### Files Validator

```rust
// src/indexer/validation/validators/files.rs

pub struct FilesValidator {
    sqlite: SqliteStorage,
}

#[async_trait]
impl Validator for FilesValidator {
    async fn validate(&self, scope: ValidationScope) -> Result<Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        
        // Check 1: Orphaned files (in DB but deleted from disk)
        issues.extend(self.check_orphaned_files().await?);
        
        // Check 2: Missing files (on disk but not in DB)
        issues.extend(self.check_missing_files().await?);
        
        // Check 3: Files with no symbols (indexed but empty symbols table)
        issues.extend(self.check_files_without_symbols().await?);
        
        // Check 4: Files with failed indexing
        issues.extend(self.check_failed_files().await?);
        
        Ok(issues)
    }
    
    fn name(&self) -> &str {
        "files"
    }
    
    fn estimated_duration(&self) -> Duration {
        Duration::from_secs(2)
    }
    
    fn supports_incremental(&self) -> bool {
        true
    }
    
    async fn get_validation_counts(&self) -> Result<ValidationCounts> {
        let files_checked = sqlx::query_scalar!("SELECT COUNT(*) FROM files")
            .fetch_one(&self.sqlite.pool)
            .await? as usize;
        
        Ok(ValidationCounts {
            files_checked,
            ..Default::default()
        })
    }
}

impl FilesValidator {
    async fn check_orphaned_files(&self) -> Result<Vec<ValidationIssue>> {
        let files = sqlx::query!(
            "SELECT id, path FROM files WHERE index_status = 'completed'"
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        let mut orphaned = Vec::new();
        
        for file in files {
            if !tokio::fs::try_exists(&file.path).await? {
                orphaned.push(AffectedItem {
                    item_type: "file".into(),
                    item_id: file.id.to_string(),
                    item_path: Some(file.path.clone()),
                    line: None,
                });
            }
        }
        
        if orphaned.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("FILES-001-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::High,
            category: IssueCategory::OrphanedData,
            message: format!("{} files in index but deleted from disk", orphaned.len()),
            details: IssueDetails {
                description: "Files exist in database but no longer on disk".into(),
                impact: "Search returns broken links, wasted storage".into(),
                root_cause: Some("Files deleted outside of gofer".into()),
                examples: orphaned.iter().take(3)
                    .filter_map(|item| item.item_path.clone())
                    .collect(),
            },
            affected_items: orphaned.clone(),
            recommendation: Recommendation {
                action: RecommendedAction::DeleteOrphanedData {
                    ids: orphaned.iter()
                        .filter_map(|item| item.item_id.parse::<i64>().ok())
                        .collect(),
                },
                command: Some("Auto-fixable: will delete from database".into()),
                estimated_time: Duration::from_secs(1),
            },
            auto_fixable: true,
        }])
    }
    
    async fn check_missing_files(&self) -> Result<Vec<ValidationIssue>> {
        // Scan workspace for files
        let workspace_files = self.scan_workspace_files().await?;
        
        // Get files in database
        let indexed_files: HashSet<String> = sqlx::query_scalar!("SELECT path FROM files")
            .fetch_all(&self.sqlite.pool)
            .await?
            .into_iter()
            .collect();
        
        // Find missing
        let missing: Vec<_> = workspace_files.into_iter()
            .filter(|path| !indexed_files.contains(path))
            .collect();
        
        if missing.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("FILES-002-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::High,
            category: IssueCategory::MissingData,
            message: format!("{} files on disk but not indexed", missing.len()),
            details: IssueDetails {
                description: "Files exist on disk but not in index".into(),
                impact: "Search won't find these files".into(),
                root_cause: Some("Files added while indexer was stopped".into()),
                examples: missing.iter().take(3).cloned().collect(),
            },
            affected_items: missing.iter().map(|path| AffectedItem {
                item_type: "file".into(),
                item_id: "N/A".into(),
                item_path: Some(path.clone()),
                line: None,
            }).collect(),
            recommendation: Recommendation {
                action: RecommendedAction::ReindexFiles {
                    paths: missing.clone(),
                },
                command: Some(format!("force_reindex --files {}", missing.join(" "))),
                estimated_time: Duration::from_secs(missing.len() as u64),
            },
            auto_fixable: false,  // Requires reindexing
        }])
    }
    
    async fn check_files_without_symbols(&self) -> Result<Vec<ValidationIssue>> {
        let files = sqlx::query!(
            r#"
            SELECT f.id, f.path
            FROM files f
            LEFT JOIN symbols s ON s.file_id = f.id
            WHERE f.index_status = 'completed'
            GROUP BY f.id
            HAVING COUNT(s.id) = 0
            "#
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        if files.is_empty() {
            return Ok(vec![]);
        }
        
        // Filter out files that legitimately have no symbols (e.g., empty files, configs)
        let suspicious: Vec<_> = files.into_iter()
            .filter(|f| self.should_have_symbols(&f.path))
            .collect();
        
        if suspicious.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("FILES-003-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Medium,
            category: IssueCategory::MissingData,
            message: format!("{} files indexed but have no symbols", suspicious.len()),
            details: IssueDetails {
                description: "Files are marked as indexed but contain no symbols".into(),
                impact: "Symbol search will miss these files".into(),
                root_cause: Some("Parser may have failed silently".into()),
                examples: suspicious.iter().take(3).map(|f| f.path.clone()).collect(),
            },
            affected_items: suspicious.iter().map(|f| AffectedItem {
                item_type: "file".into(),
                item_id: f.id.to_string(),
                item_path: Some(f.path.clone()),
                line: None,
            }).collect(),
            recommendation: Recommendation {
                action: RecommendedAction::ReindexFiles {
                    paths: suspicious.iter().map(|f| f.path.clone()).collect(),
                },
                command: None,
                estimated_time: Duration::from_secs(suspicious.len() as u64),
            },
            auto_fixable: false,
        }])
    }
    
    async fn check_failed_files(&self) -> Result<Vec<ValidationIssue>> {
        let failed = sqlx::query!(
            "SELECT id, path, error_message FROM files WHERE index_status = 'failed'"
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        if failed.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("FILES-004-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Critical,
            category: IssueCategory::CorruptedData,
            message: format!("{} files failed to index", failed.len()),
            details: IssueDetails {
                description: "Files have index_status = 'failed'".into(),
                impact: "These files are not searchable".into(),
                root_cause: None,
                examples: failed.iter().take(3)
                    .map(|f| format!("{}: {}", f.path, f.error_message.as_deref().unwrap_or("unknown error")))
                    .collect(),
            },
            affected_items: failed.iter().map(|f| AffectedItem {
                item_type: "file".into(),
                item_id: f.id.to_string(),
                item_path: Some(f.path.clone()),
                line: None,
            }).collect(),
            recommendation: Recommendation {
                action: RecommendedAction::Manual {
                    instructions: "Check logs for errors, fix issues, then reindex".into(),
                },
                command: None,
                estimated_time: Duration::from_secs(60),
            },
            auto_fixable: false,
        }])
    }
    
    fn should_have_symbols(&self, path: &str) -> bool {
        // Code files should have symbols
        path.ends_with(".rs") || 
        path.ends_with(".ts") || 
        path.ends_with(".js") ||
        path.ends_with(".py")
    }
    
    async fn scan_workspace_files(&self) -> Result<Vec<String>> {
        // Use existing file watcher to scan workspace
        // (implementation details omitted)
        Ok(vec![])
    }
}
```

### Symbols Validator

```rust
// src/indexer/validation/validators/symbols.rs

pub struct SymbolsValidator {
    sqlite: SqliteStorage,
}

#[async_trait]
impl Validator for SymbolsValidator {
    async fn validate(&self, _scope: ValidationScope) -> Result<Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        
        // Check 1: Symbols referencing deleted files
        issues.extend(self.check_orphaned_symbols().await?);
        
        // Check 2: Duplicate symbols
        issues.extend(self.check_duplicate_symbols().await?);
        
        // Check 3: Missing trait impls (heuristic)
        issues.extend(self.check_missing_trait_impls().await?);
        
        Ok(issues)
    }
    
    fn name(&self) -> &str {
        "symbols"
    }
    
    fn estimated_duration(&self) -> Duration {
        Duration::from_secs(3)
    }
}

impl SymbolsValidator {
    async fn check_orphaned_symbols(&self) -> Result<Vec<ValidationIssue>> {
        let orphaned = sqlx::query!(
            r#"
            SELECT s.id, s.name, s.file_id, f.path
            FROM symbols s
            LEFT JOIN files f ON s.file_id = f.id
            WHERE f.id IS NULL
            "#
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        if orphaned.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("SYMBOLS-001-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Critical,
            category: IssueCategory::BrokenReferences,
            message: format!("{} symbols reference non-existent files", orphaned.len()),
            details: IssueDetails {
                description: "Symbols exist but their files don't".into(),
                impact: "Search returns broken results, get_callers fails".into(),
                root_cause: Some("Files deleted but symbols not cleaned up".into()),
                examples: orphaned.iter().take(3)
                    .map(|s| format!("Symbol '{}' references missing file_id {}", s.name, s.file_id))
                    .collect(),
            },
            affected_items: orphaned.iter().map(|s| AffectedItem {
                item_type: "symbol".into(),
                item_id: s.id.to_string(),
                item_path: None,
                line: None,
            }).collect(),
            recommendation: Recommendation {
                action: RecommendedAction::DeleteOrphanedData {
                    ids: orphaned.iter().map(|s| s.id).collect(),
                },
                command: Some("Auto-fixable: will delete orphaned symbols".into()),
                estimated_time: Duration::from_secs(1),
            },
            auto_fixable: true,
        }])
    }
    
    async fn check_duplicate_symbols(&self) -> Result<Vec<ValidationIssue>> {
        let duplicates = sqlx::query!(
            r#"
            SELECT name, file_id, line, COUNT(*) as count
            FROM symbols
            GROUP BY name, file_id, line
            HAVING count > 1
            "#
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        if duplicates.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("SYMBOLS-002-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Low,
            category: IssueCategory::DuplicateData,
            message: format!("{} duplicate symbol entries", duplicates.len()),
            details: IssueDetails {
                description: "Same symbol indexed multiple times".into(),
                impact: "Wasted storage, duplicate search results".into(),
                root_cause: Some("Reindexing without cleanup".into()),
                examples: duplicates.iter().take(3)
                    .map(|d| format!("Symbol '{}' at line {} has {} copies", d.name, d.line, d.count))
                    .collect(),
            },
            affected_items: vec![],
            recommendation: Recommendation {
                action: RecommendedAction::Manual {
                    instructions: "Run deduplication: DELETE duplicates, keep one".into(),
                },
                command: None,
                estimated_time: Duration::from_secs(5),
            },
            auto_fixable: true,  // Can be auto-fixed
        }])
    }
    
    async fn check_missing_trait_impls(&self) -> Result<Vec<ValidationIssue>> {
        // Heuristic: search for "impl.*for" in files, compare with indexed impl symbols
        // This is a simplified check - full implementation would use AST
        
        let files_with_impls = self.find_files_with_impl_keyword().await?;
        let indexed_impls = self.count_indexed_impl_symbols().await?;
        
        // If we found more impl keywords than indexed impl symbols, something's missing
        if files_with_impls.len() > indexed_impls {
            Ok(vec![ValidationIssue {
                id: format!("SYMBOLS-003-{}", Uuid::new_v4().simple()),
                severity: IssueSeverity::High,
                category: IssueCategory::MissingData,
                message: format!("Potentially {} missing trait impls", files_with_impls.len() - indexed_impls),
                details: IssueDetails {
                    description: "Files contain 'impl' keyword but few impl symbols indexed".into(),
                    impact: "find_trait_impls returns incomplete results".into(),
                    root_cause: Some("Parser may not detect all impl blocks".into()),
                    examples: files_with_impls.iter().take(3).cloned().collect(),
                },
                affected_items: files_with_impls.iter().map(|path| AffectedItem {
                    item_type: "file".into(),
                    item_id: "N/A".into(),
                    item_path: Some(path.clone()),
                    line: None,
                }).collect(),
                recommendation: Recommendation {
                    action: RecommendedAction::ReindexFiles {
                        paths: files_with_impls,
                    },
                    command: None,
                    estimated_time: Duration::from_secs(3),
                },
                auto_fixable: false,
            }])
        } else {
            Ok(vec![])
        }
    }
    
    async fn find_files_with_impl_keyword(&self) -> Result<Vec<String>> {
        // Simplified: would use proper grep
        Ok(vec![])
    }
    
    async fn count_indexed_impl_symbols(&self) -> Result<usize> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM symbols WHERE kind = 'impl'"
        )
        .fetch_one(&self.sqlite.pool)
        .await? as usize;
        
        Ok(count)
    }
}
```

### Vector Validator

```rust
// src/indexer/validation/validators/vectors.rs

pub struct VectorValidator {
    sqlite: SqliteStorage,
    lance: LanceStorage,
}

#[async_trait]
impl Validator for VectorValidator {
    async fn validate(&self, _scope: ValidationScope) -> Result<Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        
        // Check 1: Chunks without vectors
        issues.extend(self.check_missing_vectors().await?);
        
        // Check 2: Outdated vectors (chunk modified, vector not regenerated)
        issues.extend(self.check_outdated_vectors().await?);
        
        // Check 3: Corrupted vectors (wrong dimension)
        issues.extend(self.check_corrupted_vectors().await?);
        
        Ok(issues)
    }
    
    fn name(&self) -> &str {
        "vectors"
    }
    
    fn estimated_duration(&self) -> Duration {
        Duration::from_secs(2)
    }
}

impl VectorValidator {
    async fn check_missing_vectors(&self) -> Result<Vec<ValidationIssue>> {
        // Find chunks in SQLite that don't have vectors in LanceDB
        let chunks = sqlx::query!("SELECT id, file_id, content FROM chunks")
            .fetch_all(&self.sqlite.pool)
            .await?;
        
        let mut missing = Vec::new();
        
        for chunk in chunks {
            if !self.lance.has_vector(chunk.id).await? {
                missing.push(chunk.id);
            }
        }
        
        if missing.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("VECTORS-001-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Medium,
            category: IssueCategory::MissingData,
            message: format!("{} chunks have no embeddings", missing.len()),
            details: IssueDetails {
                description: "Chunks exist but embeddings not generated".into(),
                impact: "Semantic search won't find these chunks".into(),
                root_cause: Some("Embedder may have failed or not run yet".into()),
                examples: vec![],
            },
            affected_items: missing.iter().map(|id| AffectedItem {
                item_type: "chunk".into(),
                item_id: id.to_string(),
                item_path: None,
                line: None,
            }).collect(),
            recommendation: Recommendation {
                action: RecommendedAction::RegenerateEmbeddings {
                    chunk_ids: missing.clone(),
                },
                command: Some("Auto-fixable: will queue for re-embedding".into()),
                estimated_time: Duration::from_secs(missing.len() as u64 / 10),  // ~10 chunks/sec
            },
            auto_fixable: true,
        }])
    }
    
    async fn check_outdated_vectors(&self) -> Result<Vec<ValidationIssue>> {
        // Find chunks where file.updated_at > chunk.embedded_at
        let outdated = sqlx::query!(
            r#"
            SELECT c.id, c.file_id, f.path, f.updated_at as file_updated, c.embedded_at
            FROM chunks c
            JOIN files f ON c.file_id = f.id
            WHERE f.updated_at > c.embedded_at
            "#
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        if outdated.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("VECTORS-002-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Medium,
            category: IssueCategory::OutdatedData,
            message: format!("{} chunks have outdated embeddings", outdated.len()),
            details: IssueDetails {
                description: "File modified but embeddings not regenerated".into(),
                impact: "Semantic search may return stale results".into(),
                root_cause: Some("File watcher missed change or embedder backlog".into()),
                examples: outdated.iter().take(3)
                    .map(|c| format!("{} modified at {:?}, embedding from {:?}", 
                        c.path, c.file_updated, c.embedded_at))
                    .collect(),
            },
            affected_items: outdated.iter().map(|c| AffectedItem {
                item_type: "chunk".into(),
                item_id: c.id.to_string(),
                item_path: Some(c.path.clone()),
                line: None,
            }).collect(),
            recommendation: Recommendation {
                action: RecommendedAction::RegenerateEmbeddings {
                    chunk_ids: outdated.iter().map(|c| c.id).collect(),
                },
                command: Some("Auto-fixable: will regenerate embeddings".into()),
                estimated_time: Duration::from_secs(outdated.len() as u64 / 10),
            },
            auto_fixable: true,
        }])
    }
    
    async fn check_corrupted_vectors(&self) -> Result<Vec<ValidationIssue>> {
        // Check if all vectors have the expected dimension
        let expected_dim = self.lance.get_vector_dimension().await?;
        let vectors_with_wrong_dim = self.lance.find_vectors_with_wrong_dimension(expected_dim).await?;
        
        if vectors_with_wrong_dim.is_empty() {
            return Ok(vec![]);
        }
        
        Ok(vec![ValidationIssue {
            id: format!("VECTORS-003-{}", Uuid::new_v4().simple()),
            severity: IssueSeverity::Critical,
            category: IssueCategory::CorruptedData,
            message: format!("{} vectors have wrong dimension", vectors_with_wrong_dim.len()),
            details: IssueDetails {
                description: format!("Vectors don't match expected dimension {}", expected_dim),
                impact: "Vector search will crash or return incorrect results".into(),
                root_cause: Some("Model changed or data corruption".into()),
                examples: vec![],
            },
            affected_items: vec![],
            recommendation: Recommendation {
                action: RecommendedAction::RebuildIndex,
                command: Some("force_reindex --embeddings-only".into()),
                estimated_time: Duration::from_secs(120),
            },
            auto_fixable: false,  // Requires full rebuild
        }])
    }
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_files_validator_orphaned() {
        let (sqlite, _lance) = setup_test_db().await;
        
        // Insert file that doesn't exist on disk
        sqlx::query!(
            "INSERT INTO files (path, index_status) VALUES ('nonexistent.rs', 'completed')"
        )
        .execute(&sqlite.pool)
        .await
        .unwrap();
        
        let validator = FilesValidator::new(sqlite);
        let issues = validator.validate(ValidationScope::Full).await.unwrap();
        
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::OrphanedData);
        assert!(issues[0].auto_fixable);
    }
    
    #[tokio::test]
    async fn test_symbols_validator_orphaned() {
        let (sqlite, _lance) = setup_test_db().await;
        
        // Insert symbol with non-existent file_id
        sqlx::query!(
            "INSERT INTO symbols (name, kind, file_id, line) VALUES ('orphan', 'function', 9999, 10)"
        )
        .execute(&sqlite.pool)
        .await
        .unwrap();
        
        let validator = SymbolsValidator::new(sqlite);
        let issues = validator.validate(ValidationScope::Full).await.unwrap();
        
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, IssueSeverity::Critical);
        assert!(issues[0].auto_fixable);
    }
    
    #[tokio::test]
    async fn test_auto_fix() {
        let (sqlite, lance) = setup_test_db().await;
        
        // Create fixable issue
        create_orphaned_symbol(&sqlite).await;
        
        let runner = ValidationRunner::new(sqlite.clone(), lance);
        let report = runner.validate_and_fix(ValidationScope::Full).await.unwrap();
        
        assert!(report.auto_fix_results.is_some());
        let fix_results = report.auto_fix_results.unwrap();
        assert_eq!(fix_results.fixed_issues, 1);
        assert_eq!(fix_results.failed_fixes, 0);
        
        // Verify symbol was deleted
        let count = sqlx::query_scalar!("SELECT COUNT(*) FROM symbols WHERE file_id = 9999")
            .fetch_one(&sqlite.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
```

---

## 📈 Success Metrics

- ✅ Finds all types of index issues
- ✅ Auto-fix resolves 80%+ of issues successfully
- ✅ Health score correlates with actual index quality
- ⏱️ Full validation completes in < 10 seconds
- ✅ Zero false positives in issue detection

---

## 📚 Usage Examples

```typescript
// Basic validation
const report = await gofer.validate_index();

if (report.summary.critical_issues > 0) {
  console.error('CRITICAL issues found!');
  report.issues
    .filter(i => i.severity === 'Critical')
    .forEach(issue => {
      console.error(`- ${issue.message}`);
      console.error(`  Fix: ${issue.recommendation.command}`);
    });
}

// Auto-fix
const report = await gofer.validate_index({ fix_auto_fixable: true });
console.log(`Fixed ${report.auto_fix_results.fixed_issues} issues automatically`);

// Incremental validation (only changed files)
const report = await gofer.validate_index({ scope: 'incremental' });
```

---

## ✅ Acceptance Criteria

- [ ] All validators implemented and tested
- [ ] Auto-fix works for 80%+ of issues
- [ ] Health score calculation is accurate
- [ ] Response time < 10 seconds for full validation
- [ ] All unit tests pass
- [ ] Documentation complete

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
