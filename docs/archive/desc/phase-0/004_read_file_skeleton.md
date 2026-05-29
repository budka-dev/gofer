# Feature: read_file_skeleton - Token-эффективное чтение

**ID:** PHASE0-004  
**Priority:** 🔥🔥🔥 Critical  
**Effort:** 1 день  
**Status:** Not Started  
**Phase:** 0 (Foundation)  
**Depends On:** None (uses existing skeleton.rs)

---

## 📋 Описание

MCP tool для чтения файлов в "skeleton" режиме - только сигнатуры функций, определения типов, imports и doc comments, без тел функций. Экономит 3-5× токенов при сохранении понимания структуры кода.

### Проблема

**Сейчас:**
- `read_file()` возвращает весь файл целиком
- AI читает 6000 токенов, использует 1500
- Большинство тел функций не нужны для понимания архитектуры
- 70% токенов тратится на implementation details

**Пример:**
```rust
// Full file: 300 lines, 6000 tokens
pub async fn process_data(input: Vec<u8>) -> Result<ProcessedData> {
    // 50 lines of implementation
    let mut result = Vec::new();
    for chunk in input.chunks(1024) {
        // ... 40 more lines
    }
    Ok(ProcessedData { result })
}
```

AI нужно знать:
- Что функция существует ✅
- Её сигнатуру ✅
- Doc comment ✅
- **Не нужно:** 50 строк implementation ❌

### Решение

Skeleton mode возвращает:
```rust
// Skeleton: 50 lines, 1200 tokens (5× экономия)
use std::collections::HashMap;

/// Processes input data and returns structured result
pub async fn process_data(input: Vec<u8>) -> Result<ProcessedData> { /* ... */ }

pub struct ProcessedData {
    pub result: Vec<u8>,
}
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ 3-5× экономия токенов при чтении файлов
- ✅ Сохранить все важное: signatures, types, docs, imports
- ✅ Поддержка всех языков (Rust, TypeScript, Python, Go)
- ✅ Fast (< 100ms для файла в 1000 строк)

### Non-Goals
- ❌ Не заменяет полное чтение (иногда нужны детали)
- ❌ Не анализирует логику (это не task)
- ❌ Не делает semantic understanding (просто парсинг)

---

## 🏗️ Архитектура

### Components

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│     read_file_skeleton()                │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Skeletonizer   │
        │    (existing)   │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│Tree-     │ │Lang  │ │  Cache    │
│Sitter    │ │Rules │ │           │
└──────────┘ └──────┘ └───────────┘
```

### Skeletonization Process

```
1. Read file → 2. Parse AST → 3. Extract nodes → 4. Format output
   ↓              ↓              ↓                ↓
File content   Tree-sitter   Keep:             Formatted
               AST           - imports          skeleton
                             - types            with /* ... */
                             - signatures       placeholders
                             - doc comments
                             
                             Remove:
                             - function bodies
                             - impl details
```

---

## 🔧 API Specification

### MCP Tool Definition

```json
{
  "name": "read_file_skeleton",
  "description": "Read file in skeleton mode (signatures only, no function bodies). Saves 3-5× tokens.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file_path": {
        "type": "string",
        "description": "Path to file (relative to project root)"
      },
      "include_private": {
        "type": "boolean",
        "default": false,
        "description": "Include private/internal items"
      },
      "include_tests": {
        "type": "boolean",
        "default": false,
        "description": "Include test functions"
      }
    },
    "required": ["file_path"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct SkeletonResponse {
    pub file_path: String,
    pub language: String,
    pub skeleton_content: String,
    pub stats: SkeletonStats,
}

#[derive(Serialize)]
pub struct SkeletonStats {
    pub original_lines: usize,
    pub original_chars: usize,
    pub skeleton_lines: usize,
    pub skeleton_chars: usize,
    pub reduction_percent: f32,
    pub items_kept: ItemCounts,
}

#[derive(Serialize)]
pub struct ItemCounts {
    pub imports: usize,
    pub types: usize,
    pub functions: usize,
    pub constants: usize,
    pub comments: usize,
}
```

### Example Response

```json
{
  "file_path": "src/indexer/parser.rs",
  "language": "rust",
  "skeleton_content": "use tree_sitter::{Parser, Tree};\nuse anyhow::Result;\n\n/// Main parser for code files\npub struct CodeParser {\n    parser: Parser,\n}\n\nimpl CodeParser {\n    pub fn new() -> Self { /* ... */ }\n    \n    pub async fn parse(&self, path: &str, content: &str) -> Result<Vec<Symbol>> { /* ... */ }\n    \n    fn extract_symbols(&self, tree: &Tree) -> Vec<Symbol> { /* ... */ }\n}",
  "stats": {
    "original_lines": 487,
    "original_chars": 15234,
    "skeleton_lines": 98,
    "skeleton_chars": 3456,
    "reduction_percent": 77.3,
    "items_kept": {
      "imports": 5,
      "types": 3,
      "functions": 12,
      "constants": 2,
      "comments": 8
    }
  }
}
```

---

## 💻 Implementation Details

### Reuse Existing Skeletonizer

Good news: уже реализовано в `src/indexer/parser/skeleton.rs`!

```rust
// src/indexer/parser/skeleton.rs (existing)
pub fn skeletonize_rust(source: &str) -> Result<String> { /* implemented */ }
pub fn skeletonize_typescript(source: &str) -> Result<String> { /* implemented */ }
pub fn skeletonize_python(source: &str) -> Result<String> { /* implemented */ }
```

Нужно только обернуть в MCP tool.

### MCP Tool Handler

```rust
// src/daemon/tools/read_skeleton.rs

use crate::indexer::parser::skeleton::{skeletonize_rust, skeletonize_typescript, skeletonize_python};
use crate::storage::SqliteStorage;
use serde_json::{Map, Value};

pub async fn handle_read_file_skeleton(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
) -> Result<Value> {
    // Parse arguments
    let file_path = args.get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing file_path"))?;
    
    let include_private = args.get("include_private")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    let include_tests = args.get("include_tests")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    // Resolve absolute path
    let workspace_root = sqlite.get_workspace_root().await?;
    let absolute_path = workspace_root.join(file_path);
    
    // Check if file exists
    if !absolute_path.exists() {
        return Err(anyhow!("File not found: {}", file_path));
    }
    
    // Read file content
    let original_content = tokio::fs::read_to_string(&absolute_path).await?;
    let original_lines = original_content.lines().count();
    let original_chars = original_content.len();
    
    // Detect language
    let language = detect_language(file_path)?;
    
    // Skeletonize
    let skeleton_content = match language.as_str() {
        "rust" => skeletonize_rust(&original_content)?,
        "typescript" | "javascript" => skeletonize_typescript(&original_content)?,
        "python" => skeletonize_python(&original_content)?,
        "go" => skeletonize_go(&original_content)?,
        _ => {
            // Fallback: return original if language not supported
            warn!("Skeletonization not supported for language: {}", language);
            original_content.clone()
        }
    };
    
    // Apply filters
    let skeleton_content = if !include_private {
        filter_private_items(&skeleton_content, &language)?
    } else {
        skeleton_content
    };
    
    let skeleton_content = if !include_tests {
        filter_test_items(&skeleton_content, &language)?
    } else {
        skeleton_content
    };
    
    // Calculate stats
    let skeleton_lines = skeleton_content.lines().count();
    let skeleton_chars = skeleton_content.len();
    let reduction_percent = 
        (1.0 - (skeleton_chars as f32 / original_chars as f32)) * 100.0;
    
    let items_kept = count_skeleton_items(&skeleton_content, &language)?;
    
    let response = SkeletonResponse {
        file_path: file_path.to_string(),
        language,
        skeleton_content,
        stats: SkeletonStats {
            original_lines,
            original_chars,
            skeleton_lines,
            skeleton_chars,
            reduction_percent,
            items_kept,
        },
    };
    
    Ok(serde_json::to_value(response)?)
}

fn detect_language(file_path: &str) -> Result<String> {
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("No file extension"))?;
    
    Ok(match extension {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        _ => "unknown",
    }.to_string())
}

fn filter_private_items(content: &str, language: &str) -> Result<String> {
    match language {
        "rust" => {
            // Remove lines starting with "fn " or "struct " without "pub"
            let lines: Vec<&str> = content.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with("fn ") || 
                       trimmed.starts_with("struct ") ||
                       trimmed.starts_with("enum ") {
                        // Keep only if has "pub"
                        line.contains("pub ")
                    } else {
                        // Keep all other lines
                        true
                    }
                })
                .collect();
            Ok(lines.join("\n"))
        }
        "typescript" | "javascript" => {
            // Remove items without "export"
            let lines: Vec<&str> = content.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with("function ") ||
                       trimmed.starts_with("class ") ||
                       trimmed.starts_with("interface ") ||
                       trimmed.starts_with("type ") {
                        line.contains("export ")
                    } else {
                        true
                    }
                })
                .collect();
            Ok(lines.join("\n"))
        }
        "python" => {
            // Remove functions/classes starting with "_"
            let lines: Vec<&str> = content.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with("def ") {
                        // Extract function name
                        if let Some(name_start) = trimmed.find("def ") {
                            let name_part = &trimmed[name_start + 4..];
                            if let Some(paren) = name_part.find('(') {
                                let name = &name_part[..paren];
                                // Keep if doesn't start with _
                                !name.starts_with('_')
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    } else if trimmed.starts_with("class ") {
                        // Extract class name
                        if let Some(name_start) = trimmed.find("class ") {
                            let name_part = &trimmed[name_start + 6..];
                            if let Some(colon_or_paren) = name_part.find(&[':', '('][..]) {
                                let name = &name_part[..colon_or_paren].trim();
                                !name.starts_with('_')
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                })
                .collect();
            Ok(lines.join("\n"))
        }
        _ => Ok(content.to_string()),
    }
}

fn filter_test_items(content: &str, language: &str) -> Result<String> {
    match language {
        "rust" => {
            // Remove #[test] and #[cfg(test)] blocks
            let lines: Vec<&str> = content.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.contains("#[test]") &&
                    !trimmed.contains("#[cfg(test)]") &&
                    !trimmed.starts_with("mod tests")
                })
                .collect();
            Ok(lines.join("\n"))
        }
        "typescript" | "javascript" => {
            // Remove describe/it/test blocks
            let lines: Vec<&str> = content.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("describe(") &&
                    !trimmed.starts_with("it(") &&
                    !trimmed.starts_with("test(")
                })
                .collect();
            Ok(lines.join("\n"))
        }
        "python" => {
            // Remove functions starting with "test_"
            let lines: Vec<&str> = content.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed.starts_with("def test_") {
                        false
                    } else {
                        true
                    }
                })
                .collect();
            Ok(lines.join("\n"))
        }
        _ => Ok(content.to_string()),
    }
}

fn count_skeleton_items(content: &str, language: &str) -> Result<ItemCounts> {
    let mut counts = ItemCounts {
        imports: 0,
        types: 0,
        functions: 0,
        constants: 0,
        comments: 0,
    };
    
    for line in content.lines() {
        let trimmed = line.trim();
        
        match language {
            "rust" => {
                if trimmed.starts_with("use ") {
                    counts.imports += 1;
                } else if trimmed.contains("struct ") || trimmed.contains("enum ") || trimmed.contains("type ") {
                    counts.types += 1;
                } else if trimmed.contains("fn ") {
                    counts.functions += 1;
                } else if trimmed.starts_with("const ") || trimmed.starts_with("static ") {
                    counts.constants += 1;
                } else if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    counts.comments += 1;
                }
            }
            "typescript" | "javascript" => {
                if trimmed.starts_with("import ") {
                    counts.imports += 1;
                } else if trimmed.contains("interface ") || trimmed.contains("type ") || trimmed.contains("class ") {
                    counts.types += 1;
                } else if trimmed.contains("function ") || trimmed.contains("=>") {
                    counts.functions += 1;
                } else if trimmed.starts_with("const ") || trimmed.starts_with("let ") {
                    counts.constants += 1;
                } else if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                    counts.comments += 1;
                }
            }
            "python" => {
                if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                    counts.imports += 1;
                } else if trimmed.starts_with("class ") {
                    counts.types += 1;
                } else if trimmed.starts_with("def ") {
                    counts.functions += 1;
                } else if trimmed.starts_with("#") {
                    counts.comments += 1;
                }
            }
            _ => {}
        }
    }
    
    Ok(counts)
}
```

### Additional Tools (Nice to Have)

```rust
// Read specific function with context
pub async fn read_function_context(
    file_path: &str,
    function_name: &str,
) -> Result<FunctionContext> {
    // Returns:
    // - Function full code (with body)
    // - Imports used by function
    // - Type definitions referenced
}

// Read types only
pub async fn read_types_only(file_path: &str) -> Result<String> {
    // Returns only type definitions (struct, enum, interface, type aliases)
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
    async fn test_skeletonize_rust() {
        let input = r#"
use std::collections::HashMap;

/// A test struct
pub struct MyStruct {
    pub field: String,
}

impl MyStruct {
    pub fn new() -> Self {
        Self {
            field: String::new(),
        }
    }
    
    pub fn process(&self) -> Result<String> {
        let mut result = String::new();
        for c in self.field.chars() {
            result.push(c);
        }
        Ok(result)
    }
}
"#;
        
        let skeleton = skeletonize_rust(input).unwrap();
        
        // Should contain signatures
        assert!(skeleton.contains("pub fn new() -> Self"));
        assert!(skeleton.contains("pub fn process(&self) -> Result<String>"));
        
        // Should not contain implementation
        assert!(!skeleton.contains("String::new()"));
        assert!(!skeleton.contains("for c in"));
        
        // Should contain types and imports
        assert!(skeleton.contains("use std::collections::HashMap"));
        assert!(skeleton.contains("pub struct MyStruct"));
    }
    
    #[tokio::test]
    async fn test_filter_private() {
        let input = r#"
pub fn public_fn() { /* ... */ }
fn private_fn() { /* ... */ }
pub struct PublicStruct {}
struct PrivateStruct {}
"#;
        
        let filtered = filter_private_items(input, "rust").unwrap();
        
        assert!(filtered.contains("pub fn public_fn"));
        assert!(!filtered.contains("fn private_fn"));
        assert!(filtered.contains("pub struct PublicStruct"));
        assert!(!filtered.contains("struct PrivateStruct"));
    }
    
    #[tokio::test]
    async fn test_token_savings() {
        let input = tokio::fs::read_to_string("src/indexer/parser.rs")
            .await
            .unwrap();
        
        let skeleton = skeletonize_rust(&input).unwrap();
        
        let original_chars = input.len();
        let skeleton_chars = skeleton.len();
        let reduction = (1.0 - (skeleton_chars as f32 / original_chars as f32)) * 100.0;
        
        // Should save at least 60%
        assert!(reduction > 60.0);
        
        println!("Original: {} chars", original_chars);
        println!("Skeleton: {} chars", skeleton_chars);
        println!("Reduction: {:.1}%", reduction);
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_mcp_tool_read_skeleton() {
    let sqlite = setup_test_db().await;
    
    let mut args = Map::new();
    args.insert("file_path".into(), Value::String("src/main.rs".into()));
    
    let response = handle_read_file_skeleton(&args, &sqlite)
        .await
        .unwrap();
    
    let resp: SkeletonResponse = serde_json::from_value(response).unwrap();
    
    assert_eq!(resp.file_path, "src/main.rs");
    assert_eq!(resp.language, "rust");
    assert!(resp.stats.reduction_percent > 50.0);
    assert!(resp.skeleton_content.contains("fn main"));
}
```

---

## 📈 Success Metrics

### Token Savings
- **Target:** 70-80% reduction
- **Minimum:** 60% reduction
- **Measured:** For files > 100 lines

### Performance
- ⏱️ Skeletonization: < 50ms for 1000-line file
- ⏱️ Total response time: < 100ms
- 💾 No memory overhead (streaming)

### Quality
- ✅ All public APIs visible
- ✅ All type definitions present
- ✅ All doc comments preserved
- ✅ Imports complete
- ❌ No implementation details leaked

---

## 📚 Usage Examples

### Basic Usage

```typescript
// AI reading file structure
const skeleton = await gofer.read_file_skeleton({
  file_path: "src/indexer/parser.rs"
});

console.log(`Original: ${skeleton.stats.original_lines} lines`);
console.log(`Skeleton: ${skeleton.stats.skeleton_lines} lines`);
console.log(`Saved: ${skeleton.stats.reduction_percent}% tokens`);
console.log(skeleton.skeleton_content);
```

### Public APIs Only

```typescript
// Only exported items
const skeleton = await gofer.read_file_skeleton({
  file_path: "src/api/routes.ts",
  include_private: false
});

// Use for API documentation generation
```

### Compare with Full Read

```typescript
// Skeleton first (fast, cheap)
const skeleton = await gofer.read_file_skeleton({
  file_path: "src/complex_module.rs"
});

// Analyze structure
if (needsImplementationDetails(skeleton)) {
  // Full read (expensive)
  const full = await gofer.read_file({
    file_path: "src/complex_module.rs"
  });
}
```

### Workflow: Incremental Detail

```typescript
// 1. Get skeleton (1200 tokens)
const skeleton = await gofer.read_file_skeleton({ file_path: "src/auth.rs" });

// 2. Identify interesting function
// "Hmm, `verify_token` looks relevant"

// 3. Get full function with context (300 tokens)
const funcContext = await gofer.read_function_context({
  file_path: "src/auth.rs",
  function_name: "verify_token"
});

// Total: 1500 tokens (vs 6000 for full file = 75% savings)
```

---

## 🔄 Language Support

### Supported
- ✅ **Rust** - Full support (already implemented)
- ✅ **TypeScript** - Full support (already implemented)
- ✅ **JavaScript** - Full support (already implemented)
- ✅ **Python** - Full support (already implemented)

### Planned
- 🟡 **Go** - Easy (similar to Rust)
- 🟡 **Java** - Medium (verbose, but structured)
- 🟡 **C/C++** - Medium (headers help)

### Fallback
For unsupported languages, return original content with warning.

---

## 🎯 Optimization Opportunities

### 1. Caching
Cache skeleton results keyed by (file_path, mtime):
```rust
struct SkeletonCache {
    cache: LruCache<(String, SystemTime), String>,
}
```

**Impact:** 2nd read of same file is instant

### 2. Parallel Processing
Skeletonize multiple files in parallel:
```rust
pub async fn read_files_skeleton(paths: Vec<String>) -> Result<Vec<SkeletonResponse>>
```

**Impact:** Batch operations 5× faster

### 3. Streaming
Stream skeleton output for large files:
```rust
pub async fn read_file_skeleton_stream(path: &str) -> impl Stream<Item = String>
```

**Impact:** Lower memory usage, faster TTFB

---

## 🐛 Edge Cases

### 1. Macros in Rust
**Problem:** Macro invocations can look like code
```rust
my_macro! {
    fn generated_function() {
        // 100 lines
    }
}
```

**Solution:** Replace entire macro invocation with `my_macro! { /* ... */ }`

### 2. Nested Functions (Python)
**Problem:** Functions inside functions
```python
def outer():
    def inner():
        # nested implementation
```

**Solution:** Keep all function signatures, replace bodies

### 3. Multi-line Signatures
**Problem:** Signatures spanning multiple lines
```rust
pub async fn very_long_function_name(
    param1: VeryLongTypeName,
    param2: AnotherLongType,
) -> Result<ComplexReturnType>
```

**Solution:** Preserve entire signature, only replace body

### 4. Doc Comments with Code Examples
**Problem:** Doc comments contain code examples
```rust
/// Example:
/// ```rust
/// let x = process();  // 20 lines of example
/// ```
pub fn process() { /* ... */ }
```

**Solution:** Keep all doc comments (they're valuable context)

---

## 📖 Related Documentation

- `../architecture/parser.md` - Parser architecture
- `skeleton.rs` - Existing skeletonizer implementation
- `005_read_function_context.md` - Function-level reading (future)

---

## ✅ Acceptance Criteria

- [ ] MCP tool `read_file_skeleton` is callable
- [ ] Returns valid JSON matching schema
- [ ] Token reduction >= 60% for typical files
- [ ] All public APIs visible in skeleton
- [ ] All doc comments preserved
- [ ] Response time < 100ms
- [ ] Supports Rust, TypeScript, Python, JavaScript
- [ ] Filter options work (private, tests)
- [ ] All unit tests pass
- [ ] Integration test passes
- [ ] Documentation complete

---

## 🚀 Rollout Plan

### Day 1: Core Implementation (4 hours)
- [ ] MCP tool handler
- [ ] Argument parsing and validation
- [ ] Integration with existing skeletonizer
- [ ] Basic testing

### Day 1: Filtering & Stats (2 hours)
- [ ] Implement `filter_private_items`
- [ ] Implement `filter_test_items`
- [ ] Calculate stats (reduction %, counts)

### Day 1: Testing & Polish (2 hours)
- [ ] Unit tests
- [ ] Integration tests
- [ ] Edge case handling
- [ ] Documentation

**Total:** 8 hours = 1 day

---

## 💡 Future Enhancements

1. **Smart Context Detection**
   - Auto-detect when full read is needed
   - Hybrid: skeleton + selected functions

2. **Customizable Skeletonization**
   - Keep certain function bodies
   - Remove certain types
   - Configurable via `.gofer/skeleton-config.toml`

3. **Semantic Skeleton**
   - Group related items
   - Show call graph in skeleton
   - Highlight "important" functions

4. **Diff-aware Skeleton**
   - Show only changed signatures
   - Compare skeleton before/after

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Note:** This is a HIGH IMPACT feature. Just wrapping existing `skeleton.rs` gives immediate 3-5× token savings across ALL file reads!
