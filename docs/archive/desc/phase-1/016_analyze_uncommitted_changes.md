# Feature: analyze_uncommitted_changes - Анализ текущих изменений

**ID:** PHASE1-016  
**Priority:** 🔥🔥🔥 High  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 1 (Runtime Context - Real-time Change Impact)

---

## 📋 Описание

Анализ несохраненных изменений в git working directory. Показывает impact незакоммиченных правок: какие функции затронуты, кто вызывает измененный код, риски breaking changes.

### Проблема

**AI не видит текущие изменения:**
```
Developer: меняет function signature
AI: "Какие тесты нужно обновить?"
→ Без анализа uncommitted changes AI не знает что изменилось

Developer: добавляет новое поле в struct
AI: "Кто использует эту структуру?"
→ AI видит только committed версию, не текущие правки
```

### Решение

```typescript
const impact = await gofer.analyze_uncommitted_changes();

// Returns:
// Modified: authenticate() - signature changed
// Affected callers: 12 locations
// Broken references: 3 (need fix)
// Test coverage: 8/12 callers have tests
// Risk: HIGH (public API change)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Parse git diff (staged + unstaged)
- ✅ Identify modified symbols
- ✅ Find affected callers
- ✅ Detect broken references
- ✅ Assess risk level
- ✅ Test coverage delta

### Non-Goals
- ❌ Не automatic fix broken references
- ❌ Не commit changes
- ❌ Не run tests

---

## 🏗️ Архитектура

```
┌─────────────────────────────────────────┐
│  analyze_uncommitted_changes()          │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │   Git Diff      │
        │   Parser        │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬────────────┐
     │           │           │            │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐ ┌───▼────┐
│ Symbol   │ │Caller│ │Reference  │ │  Risk  │
│ Analyzer │ │Finder│ │ Checker   │ │Assessor│
└──────────┘ └──────┘ └───────────┘ └────────┘
```

---

## 📊 Data Model

### MCP Tool Definition

```json
{
  "name": "analyze_uncommitted_changes",
  "description": "Анализ impact несохраненных изменений (git diff)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "include_unstaged": {
        "type": "boolean",
        "default": true,
        "description": "Включить unstaged changes"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct ChangeImpact {
    pub modified_symbols: Vec<ModifiedSymbol>,
    pub affected_callers: Vec<CallerLocation>,
    pub broken_references: Vec<BrokenRef>,
    pub test_coverage_delta: TestCoverageDiff,
    pub risk_level: RiskLevel,
    pub recommendations: Vec<String>,
}

#[derive(Serialize)]
pub struct ModifiedSymbol {
    pub name: String,
    pub kind: String,
    pub change_type: ChangeType,
    pub old_signature: Option<String>,
    pub new_signature: Option<String>,
    pub visibility: Visibility,
}

#[derive(Serialize)]
pub enum ChangeType {
    SignatureChanged,
    Added,
    Removed,
    BodyModified,
}

#[derive(Serialize)]
pub enum Visibility {
    Public,
    Private,
    Internal,
}

#[derive(Serialize)]
pub struct CallerLocation {
    pub file: String,
    pub line: u32,
    pub caller_function: String,
    pub needs_update: bool,
}

#[derive(Serialize)]
pub struct BrokenRef {
    pub file: String,
    pub line: u32,
    pub symbol: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct TestCoverageDiff {
    pub modified_functions_with_tests: usize,
    pub modified_functions_without_tests: usize,
    pub coverage_percent: f32,
}

#[derive(Serialize)]
pub enum RiskLevel {
    Low,      // Private changes, все тесты есть
    Medium,   // Internal changes, partial tests
    High,     // Public API changes
    Critical, // Breaking changes, no tests
}
```

---

## 💻 Implementation Details

### Step 1: Git Diff Parser

```rust
// src/tools/change_impact/diff_parser.rs

pub struct DiffParser {
    project_root: PathBuf,
}

impl DiffParser {
    pub async fn get_uncommitted_changes(
        &self,
        include_unstaged: bool
    ) -> Result<Vec<FileDiff>> {
        // Get staged changes
        let staged = Command::new("git")
            .args(&["diff", "--cached"])
            .output()?;
        
        let mut diffs = self.parse_diff(&staged.stdout)?;
        
        // Get unstaged changes if requested
        if include_unstaged {
            let unstaged = Command::new("git")
                .args(&["diff"])
                .output()?;
            
            diffs.extend(self.parse_diff(&unstaged.stdout)?);
        }
        
        Ok(diffs)
    }
    
    fn parse_diff(&self, diff_output: &[u8]) -> Result<Vec<FileDiff>> {
        let diff_str = String::from_utf8_lossy(diff_output);
        
        let mut diffs = Vec::new();
        let mut current_file = None;
        let mut current_hunks = Vec::new();
        
        for line in diff_str.lines() {
            if line.starts_with("diff --git") {
                // Save previous file
                if let Some(file) = current_file.take() {
                    diffs.push(FileDiff {
                        file,
                        hunks: std::mem::take(&mut current_hunks),
                    });
                }
                
                // Parse file path
                let parts: Vec<&str> = line.split_whitespace().collect();
                current_file = Some(parts[2].trim_start_matches("a/").to_string());
            } else if line.starts_with("@@") {
                // Parse hunk header
                let hunk = self.parse_hunk_header(line)?;
                current_hunks.push(hunk);
            } else if line.starts_with("+") || line.starts_with("-") {
                // Add line to current hunk
                if let Some(hunk) = current_hunks.last_mut() {
                    hunk.lines.push(line.to_string());
                }
            }
        }
        
        // Save last file
        if let Some(file) = current_file {
            diffs.push(FileDiff {
                file,
                hunks: current_hunks,
            });
        }
        
        Ok(diffs)
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub file: String,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub lines: Vec<String>,
}
```

### Step 2: Symbol Analyzer

```rust
// src/tools/change_impact/symbol_analyzer.rs

pub struct SymbolAnalyzer {
    sqlite: SqliteStorage,
}

impl SymbolAnalyzer {
    pub async fn analyze_modified_symbols(
        &self,
        diffs: &[FileDiff]
    ) -> Result<Vec<ModifiedSymbol>> {
        let mut modified = Vec::new();
        
        for diff in diffs {
            // Get current symbols from database
            let old_symbols = self.sqlite.get_symbols_for_file(&diff.file).await?;
            
            // Parse new version
            let new_content = self.reconstruct_file_content(diff)?;
            let new_symbols = self.parse_symbols(&diff.file, &new_content)?;
            
            // Diff symbols
            let changes = self.diff_symbols(&old_symbols, &new_symbols)?;
            modified.extend(changes);
        }
        
        Ok(modified)
    }
    
    fn diff_symbols(
        &self,
        old: &[Symbol],
        new: &[Symbol]
    ) -> Result<Vec<ModifiedSymbol>> {
        let mut modified = Vec::new();
        
        // Find changed symbols
        for new_sym in new {
            if let Some(old_sym) = old.iter().find(|s| s.name == new_sym.name) {
                if old_sym.signature != new_sym.signature {
                    modified.push(ModifiedSymbol {
                        name: new_sym.name.clone(),
                        kind: new_sym.kind.clone(),
                        change_type: ChangeType::SignatureChanged,
                        old_signature: Some(old_sym.signature.clone()),
                        new_signature: Some(new_sym.signature.clone()),
                        visibility: parse_visibility(&new_sym.modifiers),
                    });
                }
            } else {
                // New symbol
                modified.push(ModifiedSymbol {
                    name: new_sym.name.clone(),
                    kind: new_sym.kind.clone(),
                    change_type: ChangeType::Added,
                    old_signature: None,
                    new_signature: Some(new_sym.signature.clone()),
                    visibility: parse_visibility(&new_sym.modifiers),
                });
            }
        }
        
        // Find removed symbols
        for old_sym in old {
            if !new.iter().any(|s| s.name == old_sym.name) {
                modified.push(ModifiedSymbol {
                    name: old_sym.name.clone(),
                    kind: old_sym.kind.clone(),
                    change_type: ChangeType::Removed,
                    old_signature: Some(old_sym.signature.clone()),
                    new_signature: None,
                    visibility: parse_visibility(&old_sym.modifiers),
                });
            }
        }
        
        Ok(modified)
    }
}
```

### Step 3: Impact Analyzer

```rust
// src/tools/change_impact/impact_analyzer.rs

pub struct ImpactAnalyzer {
    sqlite: SqliteStorage,
}

impl ImpactAnalyzer {
    pub async fn analyze_impact(
        &self,
        modified_symbols: &[ModifiedSymbol]
    ) -> Result<ChangeImpact> {
        let mut affected_callers = Vec::new();
        let mut broken_references = Vec::new();
        
        for symbol in modified_symbols {
            // Find callers
            let callers = self.sqlite.get_callers(&symbol.name).await?;
            
            for caller in callers {
                let needs_update = match symbol.change_type {
                    ChangeType::SignatureChanged => true,
                    ChangeType::Removed => true,
                    _ => false,
                };
                
                affected_callers.push(CallerLocation {
                    file: caller.file,
                    line: caller.line,
                    caller_function: caller.function,
                    needs_update,
                });
                
                if symbol.change_type == ChangeType::Removed {
                    broken_references.push(BrokenRef {
                        file: caller.file.clone(),
                        line: caller.line,
                        symbol: symbol.name.clone(),
                        reason: "Symbol removed".into(),
                    });
                }
            }
        }
        
        // Assess risk
        let risk_level = self.assess_risk(modified_symbols, &affected_callers)?;
        
        // Check test coverage
        let test_coverage = self.check_test_coverage(modified_symbols).await?;
        
        // Generate recommendations
        let recommendations = self.generate_recommendations(
            &risk_level,
            &broken_references,
            &test_coverage
        );
        
        Ok(ChangeImpact {
            modified_symbols: modified_symbols.to_vec(),
            affected_callers,
            broken_references,
            test_coverage_delta: test_coverage,
            risk_level,
            recommendations,
        })
    }
    
    fn assess_risk(
        &self,
        symbols: &[ModifiedSymbol],
        callers: &[CallerLocation]
    ) -> Result<RiskLevel> {
        let has_breaking_changes = symbols.iter().any(|s| {
            matches!(s.change_type, ChangeType::SignatureChanged | ChangeType::Removed)
        });
        
        let has_public_changes = symbols.iter().any(|s| {
            matches!(s.visibility, Visibility::Public)
        });
        
        let has_many_callers = callers.len() > 10;
        
        let risk = match (has_breaking_changes, has_public_changes, has_many_callers) {
            (true, true, true) => RiskLevel::Critical,
            (true, true, _) => RiskLevel::High,
            (true, false, _) => RiskLevel::Medium,
            _ => RiskLevel::Low,
        };
        
        Ok(risk)
    }
}
```

---

## 📈 Success Metrics

- ✅ Detects 100% modified symbols
- ✅ Finds 95%+ affected callers
- ✅ Risk assessment accurate
- ⏱️ Response time < 3s

---

## 📚 Usage Example

```typescript
// AI checking impact before commit
const impact = await gofer.analyze_uncommitted_changes();

if (impact.risk_level === "Critical") {
  console.warn("⚠️ Breaking changes detected!");
  console.log("Affected callers:", impact.affected_callers.length);
  console.log("Broken references:", impact.broken_references.length);
  
  // Show recommendations
  impact.recommendations.forEach(rec => {
    console.log(`💡 ${rec}`);
  });
}
```

---

## ✅ Acceptance Criteria

- [ ] Parses git diff correctly
- [ ] Identifies modified symbols
- [ ] Finds affected callers
- [ ] Detects broken references
- [ ] Risk assessment accurate
- [ ] Test coverage analysis works
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** ВЫСОКИЙ - дает AI awareness о текущих изменениях в реальном времени.
