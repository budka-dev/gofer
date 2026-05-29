# Feature: smart_context_bundle - Умная сборка контекста

**ID:** PHASE1-020  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 1 (Optimization & Unified Tools)

---

## 📋 Описание

Расширение существующего `context_bundle` с интеллектуальным summary mode. Вместо полного кода dependencies возвращает AI-generated summaries и только публичные символы.

### Проблема

**Existing context_bundle:**
```
AI: context_bundle("server.rs")

Returns:
- server.rs: 500 строк (full)
- auth.rs: 300 строк (full dependency)
- db.rs: 400 строк (full dependency)
- http.rs: 200 строк (full dependency)

Total: 1400 строк, ~3500 токенов
Problem: 80% dependency code не релевантен для задачи
```

**С smart_context_bundle:**
```
AI: smart_context_bundle("server.rs", mode="summary")

Returns:
- server.rs: 500 строк (full)
- auth.rs: 50 строк (summary + exported symbols)
- db.rs: 40 строк (summary + exported symbols)  
- http.rs: 30 строк (summary + exported symbols)

Total: 620 строк, ~1200 токенов
Savings: 70-80% токенов!
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Extend context_bundle с summary mode
- ✅ AI-generated summaries для dependencies
- ✅ Include only exported symbols
- ✅ 70-80% token savings
- ✅ Keep main file full

### Non-Goals
- ❌ Не заменяет полный context_bundle
- ❌ Не рекурсивный (только 1 level deps)

---

## 🔧 API Specification

```json
{
  "name": "smart_context_bundle",
  "description": "Собрать context с summaries для dependencies. Экономит 70-80% токенов.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file": {"type": "string"},
      "mode": {
        "type": "string",
        "enum": ["full", "summary", "skeleton"],
        "default": "summary"
      },
      "depth": {
        "type": "number",
        "default": 1,
        "description": "Dependency depth"
      }
    },
    "required": ["file"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct SmartContextBundle {
    pub main_file: FileContent,
    pub dependencies: Vec<DependencySummary>,
    pub stats: BundleStats,
}

#[derive(Serialize)]
pub struct DependencySummary {
    pub file: String,
    pub summary: String,  // AI-generated
    pub exports: Vec<Symbol>,
    pub imports_from_main: Vec<Symbol>,
}

#[derive(Serialize)]
pub struct BundleStats {
    pub total_files: usize,
    pub total_lines: usize,
    pub estimated_tokens: usize,
    pub savings_percent: f32,
}
```

---

## 💻 Implementation

### Summary Generation

```rust
pub async fn smart_context_bundle(
    file: &str,
    mode: &str
) -> Result<SmartContextBundle> {
    // 1. Get main file (full)
    let main_content = read_file(file).await?;
    
    // 2. Find dependencies
    let deps = find_dependencies(file).await?;
    
    // 3. Generate summaries
    let mut dep_summaries = Vec::new();
    
    for dep_file in deps {
        match mode {
            "summary" => {
                // AI-generated summary
                let summary = generate_ai_summary(&dep_file).await?;
                let exports = extract_exported_symbols(&dep_file).await?;
                let imports = find_imports_from_main(file, &dep_file).await?;
                
                dep_summaries.push(DependencySummary {
                    file: dep_file,
                    summary,
                    exports,
                    imports_from_main: imports,
                });
            }
            "skeleton" => {
                // Use existing skeleton
                let skeleton = skeletonize_file(&dep_file).await?;
                dep_summaries.push(DependencySummary {
                    file: dep_file,
                    summary: skeleton,
                    exports: vec![],
                    imports_from_main: vec![],
                });
            }
            _ => {
                // Full mode (existing behavior)
                let content = read_file(&dep_file).await?;
                dep_summaries.push(DependencySummary {
                    file: dep_file,
                    summary: content,
                    exports: vec![],
                    imports_from_main: vec![],
                });
            }
        }
    }
    
    Ok(SmartContextBundle {
        main_file: main_content,
        dependencies: dep_summaries,
        stats: calculate_stats(&main_content, &dep_summaries),
    })
}

async fn generate_ai_summary(file: &str) -> Result<String> {
    // Check if summary exists in cache
    if let Some(cached) = get_cached_summary(file).await? {
        return Ok(cached);
    }
    
    // Generate new summary via LLM
    let content = read_file(file).await?;
    
    let prompt = format!(
        "Summarize this code file in 2-3 sentences. Focus on:\n\
         - Main purpose\n\
         - Key exported functions/types\n\
         - Dependencies\n\n\
         File: {}\n\n\
         Code:\n{}",
        file, content
    );
    
    let summary = call_llm(&prompt).await?;
    
    // Cache for future
    cache_summary(file, &summary).await?;
    
    Ok(summary)
}

async fn extract_exported_symbols(file: &str) -> Result<Vec<Symbol>> {
    // Query symbols with pub/export visibility
    let symbols = sqlx::query_as!(
        Symbol,
        r#"
        SELECT * FROM symbols
        WHERE file = ?
          AND (visibility = 'public' OR visibility = 'exported')
        "#,
        file
    )
    .fetch_all(&pool)
    .await?;
    
    Ok(symbols)
}
```

---

## 📈 Success Metrics

- ⚡ 70-80% token savings vs full bundle
- ✅ Summary quality: accurate & concise
- ⏱️ Response time: < 3s (including summary gen)

---

## 📚 Usage Example

```typescript
// Research scenario: понять как работает модуль
const bundle = await gofer.smart_context_bundle({
  file: "src/server.rs",
  mode: "summary"
});

console.log("Main file:", bundle.main_file.length, "lines");
console.log("Dependencies:", bundle.dependencies.length);

bundle.dependencies.forEach(dep => {
  console.log(`\n${dep.file}:`);
  console.log(`  Summary: ${dep.summary}`);
  console.log(`  Exports: ${dep.exports.length}`);
});

console.log("\nStats:");
console.log(`  Total tokens: ~${bundle.stats.estimated_tokens}`);
console.log(`  Savings: ${bundle.stats.savings_percent}%`);
```

---

## ✅ Acceptance Criteria

- [ ] Extends existing context_bundle
- [ ] Summary mode generates AI summaries
- [ ] Only includes exported symbols
- [ ] 70-80% token savings
- [ ] Summaries cached for reuse
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** ВЫСОКИЙ - критично для research/exploration workflows.
