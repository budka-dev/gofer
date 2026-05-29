# План исправления Phase 0 фич

**Дата создания:** 2026-02-16  
**Статус:** Требует реализации  
**Приоритет:** Критичный для релиза Phase 0

---

## Обзор проблем

Из 16 фич Phase 0:
- ✅ **8 фич полностью реализованы** (001-008)
- ⚠️ **4 фичи частично реализованы** с критическими недостатками (009, 010, 011, 013)
- ❌ **3 фичи не реализованы** (012, 014, 015)
- ✅ **1 фича реализована** (016: error_recovery)

Этот документ фокусируется на **4 частично реализованных фичах**.

---

## Feature 009: read_function_context

**Файл:** `src/daemon/tools.rs:3878-4105`  
**Статус:** 🟡 Частично реализовано (40%)  
**Спецификация:** `docs/desc/phase-0/009_read_function_context.md`

### Проблемы

#### ❌ Проблема 1: Types extraction не реализован (критично)

**Текущий код** (lines 4031-4051):
```rust
// For MVP: just note that types should be included
context_parts.push(json!({
    "section": "types",
    "note": "Type extraction in development",
    "lines": 0
}));
```

**Проблема:** Вместо извлечения типов просто заглушка! Экономия токенов не достигается.

**Решение:**
```rust
/// Extract type definitions referenced in function
async fn extract_referenced_types(
    function_node: &Node,
    content: &str,
    lang: &SupportedLanguage,
    sqlite: &SqliteStorage,
    file_path: &str
) -> Result<Vec<TypeDefinition>> {
    // 1. Parse function body for type references
    //    - Look for type_identifier nodes
    //    - Extract type names (e.g., "User", "Request")
    
    // 2. Query SQLite for type definitions
    //    SELECT * FROM symbols 
    //    WHERE kind IN ('struct', 'enum', 'interface') 
    //    AND name IN (type_names)
    
    // 3. Extract type code from file using line numbers
    //    Read file, extract lines [start_line..end_line]
    
    // 4. Return Vec<TypeDefinition>
    Ok(type_definitions)
}

// В tool_read_function_context добавить:
if include_types {
    let type_defs = extract_referenced_types(
        &function_node, 
        &content, 
        &lang, 
        ctx.sqlite, 
        &file_path
    ).await?;
    
    for type_def in type_defs {
        context_parts.push(json!({
            "section": "types",
            "name": type_def.name,
            "kind": type_def.kind,
            "code": type_def.code,
            "lines": type_def.lines
        }));
        total_lines += type_def.lines;
    }
}
```

**Приоритет:** 🔥🔥🔥 Критично  
**Время:** 4 часа  
**Файлы для изменения:** `src/daemon/tools.rs`

---

#### ❌ Проблема 2: Callees extraction неполный

**Текущий код** (lines 4069-4084):
```rust
if include_callees {
    if let Ok(references) = ctx.sqlite.get_references_by_name(function_name).await {
        let callee_names: Vec<_> = references.iter()
            .map(|r| r.target_name.as_str())
            .collect();
        if !callee_names.is_empty() {
            context_parts.push(json!({
                "section": "callees",
                "note": format!("This function calls: {}", 
                    callee_names.iter().take(5).copied().collect::<Vec<_>>().join(", ")),
                "count": callee_names.len()
            }));
        }
    }
}
```

**Проблемы:**
- Использует SQLite references вместо AST analysis
- Не извлекает **код** вызываемых функций, только names
- Не работает для local function calls (same file)

**Решение:**
```rust
/// Extract code of functions called by target function
async fn extract_callees(
    function_node: &Node,
    content: &str,
    lang: &SupportedLanguage,
    file_path: &Path,
) -> Result<Vec<CalleeFunction>> {
    // 1. Query AST for call_expression nodes inside function
    let call_query = match lang {
        Rust => "(call_expression function: (identifier) @callee)",
        TypeScript => "(call_expression function: (identifier) @callee)",
        // ...
    };
    
    // 2. Extract function names from calls
    let mut callee_names = HashSet::new();
    // ... parse and collect names
    
    // 3. Find definitions in same file
    let mut callees = Vec::new();
    for callee_name in callee_names {
        // Search for function_item with this name
        if let Some(callee_node) = find_function_in_file(callee_name, content, lang) {
            let callee_code = callee_node.utf8_text(content.as_bytes())?;
            callees.push(CalleeFunction {
                name: callee_name.to_string(),
                code: callee_code.to_string(),
                lines: callee_code.lines().count(),
            });
        }
    }
    
    Ok(callees)
}

// В tool_read_function_context:
if include_callees {
    let callees = extract_callees(&function_node, &content, &lang, &file_path).await?;
    
    for callee in callees {
        context_parts.push(json!({
            "section": "callee",
            "name": callee.name,
            "code": callee.code,
            "lines": callee.lines
        }));
        total_lines += callee.lines;
    }
}
```

**Приоритет:** 🔥🔥 Высокий  
**Время:** 3 часа  
**Файлы:** `src/daemon/tools.rs`

---

#### ❌ Проблема 3: Import filtering не работает

**Текущий код** (lines 3979-4029):
```rust
if include_imports {
    // ... query all imports
    while let Some(match_) = import_matches.next() {
        for capture in match_.captures {
            if let Ok(import_text) = capture.node.utf8_text(content.as_bytes()) {
                imports_code.push_str(import_text);
                imports_code.push('\n');
            }
        }
    }
}
```

**Проблема:** Возвращает **ВСЕ** импорты файла вместо только используемых в функции!

**Решение:**
```rust
/// Filter imports to only those used in function
fn filter_used_imports(
    all_imports: Vec<String>,
    function_code: &str,
) -> Vec<String> {
    let mut used_imports = Vec::new();
    
    // 1. Extract identifiers from function code
    let identifiers = extract_identifiers(function_code);
    
    // 2. For each import, check if any identifier matches
    for import in all_imports {
        // Parse import: "use std::collections::HashMap;"
        // Extract: ["std", "collections", "HashMap"]
        let import_items = parse_import_items(&import);
        
        // Check if any identifier in function uses this import
        if identifiers.iter().any(|id| import_items.contains(id)) {
            used_imports.push(import);
        }
    }
    
    used_imports
}

// В tool_read_function_context:
if include_imports {
    // Get ALL imports
    let all_imports = extract_all_imports(&tree, &content, &lang)?;
    
    // Filter to only used imports
    let used_imports = filter_used_imports(all_imports, function_code);
    
    if !used_imports.is_empty() {
        let imports_code = used_imports.join("\n");
        // ... add to context_parts
    }
}
```

**Приоритет:** 🔥🔥 Высокий  
**Время:** 2 часа  
**Файлы:** `src/daemon/tools.rs`

---

### Feature 009: Итого

**Общее время исправления:** 9 часов (1+ день)  
**Критичность:** Высокая - без этого фича бесполезна

**Порядок выполнения:**
1. Type extraction (4 часа) - самое критичное
2. Callees improvement (3 часа)
3. Import filtering (2 часа)

---

## Feature 010: read_types_only

**Файл:** `src/daemon/tools.rs:4108-4332`  
**Статус:** 🟡 Частично реализовано (70%)  
**Спецификация:** `docs/desc/phase-0/010_read_types_only.md`

### Проблемы

#### ⚠️ Проблема 1: Doc comments extraction неполный

**Текущий код** (lines 4266-4295):
```rust
// Look for doc comments above the type
if include_docs {
    let mut check_line = type_start_line;
    while check_line > 0 {
        check_line -= 1;
        let line_content = content.lines().nth(check_line).unwrap_or("");
        let trimmed = line_content.trim();
        
        // Check for doc comments
        if trimmed.starts_with("///") || trimmed.starts_with("/**") || 
           trimmed.starts_with("//!") || trimmed.starts_with("#[doc") ||
           trimmed.starts_with("\"\"\"") {
            continue;
        } else if trimmed.is_empty() {
            continue;
        } else {
            check_line += 1;
            break;
        }
    }
}
```

**Проблемы:**
- Простой backwards scan по строкам (неэффективно)
- Не обрабатывает multi-line `/* ... */` comments правильно
- Пропускает Rust attributes: `#[derive(Debug, Serialize)]`

**Решение:**
```rust
/// Extract doc comments and attributes using tree-sitter
fn extract_doc_comments_and_attributes(
    type_node: &Node,
    tree: &Tree,
    content: &str,
    lang: &SupportedLanguage
) -> (Option<String>, Option<String>) {
    let mut doc_comments = Vec::new();
    let mut attributes = Vec::new();
    
    // Use tree-sitter to find comment/attribute nodes before type
    let query_str = match lang {
        Rust => r#"
            (line_comment) @comment
            (block_comment) @comment
            (attribute_item) @attribute
        "#,
        TypeScript => r#"
            (comment) @comment
        "#,
        // ...
    };
    
    let query = Query::new(&lang.tree_sitter_language(), query_str)?;
    let mut cursor = QueryCursor::new();
    
    // Find all comments/attributes before type_node
    let type_start_byte = type_node.start_byte();
    let search_start = type_start_byte.saturating_sub(1000); // Look back max 1000 bytes
    
    for match_ in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
        for capture in match_.captures {
            if capture.node.end_byte() < type_start_byte 
               && capture.node.end_byte() > search_start {
                
                let text = capture.node.utf8_text(content.as_bytes())?;
                let capture_name = query.capture_names()[capture.index as usize];
                
                match capture_name {
                    "comment" if is_doc_comment(text) => {
                        doc_comments.push(text.to_string());
                    }
                    "attribute" => {
                        attributes.push(text.to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    
    (
        if doc_comments.is_empty() { None } else { Some(doc_comments.join("\n")) },
        if attributes.is_empty() { None } else { Some(attributes.join("\n")) }
    )
}

fn is_doc_comment(text: &str) -> bool {
    text.starts_with("///") 
        || text.starts_with("//!")
        || text.starts_with("/**")
        || text.starts_with("\"\"\"")
}
```

**Приоритет:** 🔥🔥 Высокий  
**Время:** 2 часа  
**Файлы:** `src/daemon/tools.rs`

---

#### ⚠️ Проблема 2: Не поддерживает Rust attributes

**Проблема:** Attributes как `#[derive(Debug, Serialize)]` критичны для понимания типов в Rust, но не включаются в output.

**Решение:** Уже включено в решение выше (`extract_doc_comments_and_attributes`)

**Использование:**
```rust
// В tool_read_types_only, при добавлении type в результат:
let (doc_comment, attributes) = extract_doc_comments_and_attributes(
    &type_node, 
    &tree, 
    &content, 
    &lang
);

let mut full_type_code = String::new();

// Add attributes first
if let Some(attrs) = attributes {
    full_type_code.push_str(&attrs);
    full_type_code.push('\n');
}

// Add doc comments
if include_docs {
    if let Some(docs) = doc_comment {
        full_type_code.push_str(&docs);
        full_type_code.push('\n');
    }
}

// Add type code
full_type_code.push_str(type_code);

types.push(json!({
    "name": type_name,
    "kind": type_kind,
    "code": full_type_code,
    "has_docs": doc_comment.is_some(),
    "has_attributes": attributes.is_some(),
    // ...
}));
```

---

### Feature 010: Итого

**Общее время исправления:** 2 часа  
**Критичность:** Средняя - работает, но качество output низкое

**Порядок выполнения:**
1. Doc comments + attributes extraction (2 часа)

---

## Feature 011: smart_file_selection

**Файл:** `src/daemon/tools.rs:4335-4558`  
**Статус:** 🟡 Частично реализовано (60%)  
**Спецификация:** `docs/desc/phase-0/011_smart_file_selection.md`

### Проблемы

#### ❌ Проблема 1: Примитивный scoring algorithm

**Текущий код** (lines 4391-4397):
```rust
// Weighted aggregate score
// Vector: 40%, Path: 20%, Symbols: 25%, Summary: 15%
let final_score = 
    vector_score * 0.4 + 
    path_score * 0.2 + 
    symbol_score * 0.25 + 
    summary_score * 0.15;
```

**Проблемы:**
- Фиксированные веса (не adaptive к типу запроса)
- Не учитывает **recency** (recently modified files more relevant)
- Не учитывает **file size** (huge files = worse UX)
- Нет confidence scoring

**Решение:**
```rust
/// Calculate relevance score with adaptive weights
fn calculate_relevance_score_v2(
    query: &str,
    file_info: &FileInfo,
    vector_score: f32,
    path_score: f32,
    symbol_score: f32,
    summary_score: f32,
) -> (f32, ScoringDetails) {
    // 1. Determine query type and adjust weights
    let weights = calculate_adaptive_weights(query);
    
    // 2. Calculate recency boost
    let recency_boost = calculate_recency_boost(file_info.last_modified);
    
    // 3. Calculate size penalty
    let size_penalty = calculate_size_penalty(file_info.size_bytes);
    
    // 4. Calculate base score
    let base_score = 
        vector_score * weights.vector + 
        path_score * weights.path + 
        symbol_score * weights.symbols + 
        summary_score * weights.summary;
    
    // 5. Apply modifiers
    let final_score = base_score * recency_boost * size_penalty;
    
    // 6. Calculate confidence
    let confidence = calculate_confidence(vector_score, path_score, symbol_score);
    
    let details = ScoringDetails {
        base_score,
        recency_boost,
        size_penalty,
        confidence,
        weights,
    };
    
    (final_score.clamp(0.0, 1.0), details)
}

/// Adaptive weights based on query analysis
fn calculate_adaptive_weights(query: &str) -> Weights {
    let query_lower = query.to_lowercase();
    
    // Pattern 1: "where is X defined?" -> prioritize symbols
    if query_lower.contains("where") || query_lower.contains("defined") {
        return Weights {
            vector: 0.25,
            path: 0.15,
            symbols: 0.50,  // Boost symbols
            summary: 0.10,
        };
    }
    
    // Pattern 2: "how does X work?" -> prioritize summary
    if query_lower.contains("how") || query_lower.contains("explain") {
        return Weights {
            vector: 0.35,
            path: 0.15,
            symbols: 0.15,
            summary: 0.35,  // Boost summary
        };
    }
    
    // Pattern 3: file path mentioned -> prioritize path
    if query.contains("/") || query.contains(".rs") || query.contains(".ts") {
        return Weights {
            vector: 0.30,
            path: 0.40,  // Boost path
            symbols: 0.20,
            summary: 0.10,
        };
    }
    
    // Default: balanced
    Weights {
        vector: 0.40,
        path: 0.20,
        symbols: 0.25,
        summary: 0.15,
    }
}

/// Recency boost: recently modified files are more relevant
fn calculate_recency_boost(last_modified: SystemTime) -> f32 {
    let age = SystemTime::now()
        .duration_since(last_modified)
        .unwrap_or_default();
    
    let days_old = age.as_secs() / 86400;
    
    match days_old {
        0..=1 => 1.15,    // Modified today/yesterday: +15%
        2..=7 => 1.05,    // This week: +5%
        8..=30 => 1.0,    // This month: no change
        31..=90 => 0.95,  // Last 3 months: -5%
        _ => 0.90,        // Older: -10%
    }
}

/// Size penalty: very large files are harder to work with
fn calculate_size_penalty(size_bytes: usize) -> f32 {
    let size_kb = size_bytes / 1024;
    
    match size_kb {
        0..=50 => 1.0,      // < 50KB: no penalty
        51..=200 => 0.98,   // 50-200KB: tiny penalty
        201..=500 => 0.95,  // 200-500KB: small penalty
        501..=1000 => 0.90, // 500KB-1MB: medium penalty
        _ => 0.85,          // > 1MB: large penalty
    }
}

/// Confidence: how confident are we in this ranking?
fn calculate_confidence(vector: f32, path: f32, symbol: f32) -> f32 {
    // High confidence if multiple signals agree
    let signals = vec![vector, path, symbol];
    let mean = signals.iter().sum::<f32>() / signals.len() as f32;
    let variance = signals.iter()
        .map(|&s| (s - mean).powi(2))
        .sum::<f32>() / signals.len() as f32;
    
    // Low variance = high confidence
    let confidence = 1.0 - variance.sqrt();
    confidence.clamp(0.0, 1.0)
}

#[derive(Debug, Serialize)]
struct Weights {
    vector: f32,
    path: f32,
    symbols: f32,
    summary: f32,
}

#[derive(Debug, Serialize)]
struct ScoringDetails {
    base_score: f32,
    recency_boost: f32,
    size_penalty: f32,
    confidence: f32,
    weights: Weights,
}
```

**Приоритет:** 🔥🔥🔥 Критично  
**Время:** 4 часа  
**Файлы:** `src/daemon/tools.rs`

---

#### ⚠️ Проблема 2: Path scoring слишком простой

**Текущий код** (lines 4456-4478):
```rust
fn calculate_path_score(query: &str, path: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let path_lower = path.to_lowercase();
    
    let mut score: f32 = 0.0;
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();
    
    for keyword in keywords {
        if path_lower.contains(keyword) {
            // Higher score for filename match vs directory
            if path_lower.split('/').last().unwrap_or("").contains(keyword) {
                score += 0.3;
            } else {
                score += 0.1;
            }
        }
    }
    
    score.min(1.0)
}
```

**Проблемы:**
- Простой substring match (не учитывает edit distance)
- Не учитывает directory hierarchy importance
- Нет stemming/normalization ("authenticate" vs "auth")

**Решение:**
```rust
fn calculate_path_score_v2(query: &str, path: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let path_lower = path.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();
    
    let mut score: f32 = 0.0;
    
    // Extract path components
    let filename = path_lower.split('/').last().unwrap_or("");
    let filename_stem = filename.split('.').next().unwrap_or("");
    let directories: Vec<&str> = path_lower.split('/').collect();
    
    for keyword in &keywords {
        // Normalize keyword (basic stemming)
        let normalized_keyword = normalize_keyword(keyword);
        
        // 1. Exact filename match (highest priority)
        if filename_stem == normalized_keyword {
            score += 0.5;
            continue;
        }
        
        // 2. Filename contains keyword
        if filename_stem.contains(&normalized_keyword) {
            // Calculate similarity ratio
            let similarity = similarity_ratio(&normalized_keyword, filename_stem);
            score += 0.3 * similarity;
            continue;
        }
        
        // 3. Important directory matches (e.g., src/auth/)
        let important_dirs = ["src", "lib", "core", "api"];
        for (idx, dir) in directories.iter().enumerate() {
            if dir.contains(&normalized_keyword) {
                // More important directories = higher score
                let importance = if important_dirs.contains(dir) { 1.2 } else { 1.0 };
                // Closer to filename = higher score
                let proximity = 1.0 / (directories.len() - idx) as f32;
                score += 0.15 * importance * proximity;
            }
        }
        
        // 4. Fuzzy match using edit distance
        if edit_distance(keyword, filename_stem) <= 2 {
            score += 0.2;
        }
    }
    
    score.min(1.0)
}

/// Normalize keyword (basic stemming)
fn normalize_keyword(word: &str) -> String {
    // Remove common suffixes
    let word = word.trim_end_matches("ing");
    let word = word.trim_end_matches("ed");
    let word = word.trim_end_matches("s");
    word.to_string()
}

/// Calculate similarity ratio (Jaro-Winkler-like)
fn similarity_ratio(a: &str, b: &str) -> f32 {
    let matches = a.chars()
        .filter(|c| b.contains(*c))
        .count();
    
    let max_len = a.len().max(b.len());
    if max_len == 0 { return 1.0; }
    
    matches as f32 / max_len as f32
}

/// Levenshtein edit distance
fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    
    if a_len == 0 { return b_len; }
    if b_len == 0 { return a_len; }
    
    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];
    
    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }
    
    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i-1] == b_chars[j-1] { 0 } else { 1 };
            matrix[i][j] = *[
                matrix[i-1][j] + 1,      // deletion
                matrix[i][j-1] + 1,      // insertion
                matrix[i-1][j-1] + cost, // substitution
            ].iter().min().unwrap();
        }
    }
    
    matrix[a_len][b_len]
}
```

**Приоритет:** 🔥🔥 Высокий  
**Время:** 3 часа  
**Файлы:** `src/daemon/tools.rs`

---

#### ⚠️ Проблема 3: Нет caching для популярных queries

**Проблема:** Каждый `smart_file_selection` запрос делает:
- Vector search (дорого)
- SQLite queries для каждого файла
- Summary fetching

Для популярных queries (e.g., "authentication", "database") можно кешировать результаты.

**Решение:**
```rust
// В src/cache.rs добавить:

impl CacheManager {
    /// Cache file selection results
    pub async fn get_file_selection(&self, query: &str, limit: usize) -> Option<String> {
        let cache_key = format!("file_selection:{}:{}", query, limit);
        let mut cache = self.search_cache.write().await;
        
        let result = cache.get(&cache_key);
        
        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.search_hits += 1;
        } else {
            stats.search_misses += 1;
        }
        
        result
    }
    
    pub async fn put_file_selection(&self, query: String, limit: usize, data: String) {
        let cache_key = format!("file_selection:{}:{}", query, limit);
        let size = data.len();
        let mut cache = self.search_cache.write().await;
        cache.put(cache_key, data, size);
    }
}

// В tool_smart_file_selection добавить:

async fn tool_smart_file_selection(args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let min_score = args.get("min_score").and_then(|v| v.as_f64()).unwrap_or(0.3) as f32;

    // Check cache first
    if let Some(cached_json) = ctx.cache.get_file_selection(query, limit).await {
        if let Ok(cached_result) = serde_json::from_str::<Value>(&cached_json) {
            return Ok(cached_result);
        }
    }

    // ... existing logic ...
    
    let result = json!({
        "files": ranked_files,
        "reasoning": reasoning,
        "total_candidates": total_candidates
    });
    
    // Store in cache
    if let Ok(result_json) = serde_json::to_string(&result) {
        ctx.cache.put_file_selection(query.to_string(), limit, result_json).await;
    }
    
    Ok(result)
}
```

**Приоритет:** 🔥 Средний  
**Время:** 1 час  
**Файлы:** `src/cache.rs`, `src/daemon/tools.rs`

---

### Feature 011: Итого

**Общее время исправления:** 8 часов (1 день)  
**Критичность:** Высокая - это ключевая фича для больших кодбаз

**Порядок выполнения:**
1. Улучшить scoring algorithm (4 часа) - самое критичное
2. Улучшить path scoring (3 часа)
3. Добавить caching (1 час)

---

## Feature 013: batch_operations

**Файл:** `src/daemon/tools.rs:4561-4653`  
**Статус:** 🔴 Критически неполно (30%)  
**Спецификация:** `docs/desc/phase-0/013_batch_operations.md`

### Проблемы

#### ❌ Проблема 1: Parallel execution НЕ РЕАЛИЗОВАН (критично!)

**Текущий код** (lines 4568-4570):
```rust
let parallel = args.get("parallel")
    .and_then(|v| v.as_bool())
    .unwrap_or(false); // Changed default to false for simplicity
```

**Далее:**
```rust
// Sequential execution for now to avoid lifetime issues
for (idx, operation) in operations.iter().enumerate() {
    // Always sequential!
}
```

**Проблема:** Parallel execution полностью отключен! Это **убивает весь смысл фичи**.

Цель фичи: 3-5× latency reduction через параллельное выполнение.  
Реальность: Sequential execution = **NO speedup**.

**Почему не реализовано:**  
Comment говорит "to avoid lifetime issues". Проблема в том, что `ToolContext` содержит `&` references:

```rust
pub struct ToolContext<'a> {
    pub sqlite: &'a SqliteStorage,
    pub lance: &'a Mutex<LanceStorage>,
    pub embedder: &'a EmbedderPool,
    // ...
}
```

Нельзя clone `&'a` references для передачи в `tokio::spawn`.

**Решение: Refactor ToolContext to use Arc**

```rust
// Шаг 1: Refactor ToolContext
pub struct ToolContext {
    pub sqlite: Arc<SqliteStorage>,
    pub lance: Arc<Mutex<LanceStorage>>,
    pub embedder: Arc<EmbedderPool>,
    pub reranker: Arc<Option<Reranker>>,
    pub root_path: Arc<PathBuf>,
    pub cache: Arc<CacheManager>,
    pub embedding_circuit: Arc<CircuitBreaker>,
    pub vector_circuit: Arc<CircuitBreaker>,
}

impl Clone for ToolContext {
    fn clone(&self) -> Self {
        Self {
            sqlite: Arc::clone(&self.sqlite),
            lance: Arc::clone(&self.lance),
            embedder: Arc::clone(&self.embedder),
            reranker: Arc::clone(&self.reranker),
            root_path: Arc::clone(&self.root_path),
            cache: Arc::clone(&self.cache),
            embedding_circuit: Arc::clone(&self.embedding_circuit),
            vector_circuit: Arc::clone(&self.vector_circuit),
        }
    }
}

// Шаг 2: Update все места где создается ToolContext
// src/daemon/state.rs
impl ProjectState {
    pub fn tool_context(&self) -> ToolContext {
        ToolContext {
            sqlite: Arc::clone(&self.sqlite),
            lance: Arc::clone(&self.lance),
            embedder: Arc::clone(&self.embedder),
            reranker: Arc::new(self.reranker.clone()),
            root_path: Arc::new(self.root_path.clone()),
            cache: Arc::clone(&self.cache),
            embedding_circuit: Arc::clone(&self.embedding_circuit),
            vector_circuit: Arc::clone(&self.vector_circuit),
        }
    }
}

// Шаг 3: Implement parallel execution
async fn tool_batch_operations(args: Value, ctx: &ToolContext) -> Result<Value> {
    let operations = args.get("operations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| goferError::InvalidParams("operations array is required".into()))?;
    
    let parallel = args.get("parallel")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // NOW default to true!
    
    let continue_on_error = args.get("continue_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let start = Instant::now();
    let results;

    if parallel {
        // Parallel execution with rate limiting
        let semaphore = Arc::new(Semaphore::new(10)); // Max 10 concurrent
        
        let tasks: Vec<_> = operations.iter()
            .enumerate()
            .map(|(idx, operation)| {
                let ctx = ctx.clone(); // NOW we can clone!
                let op = operation.clone();
                let sem = Arc::clone(&semaphore);
                
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    execute_single_operation(idx, op, &ctx).await
                })
            })
            .collect();
        
        // Await all tasks
        let mut batch_results = Vec::new();
        for task in tasks {
            match task.await {
                Ok(Ok(result)) => batch_results.push(result),
                Ok(Err(e)) => {
                    if !continue_on_error {
                        return Err(e);
                    }
                    batch_results.push(create_error_result(e));
                }
                Err(e) => {
                    return Err(anyhow!("Task join error: {}", e).into());
                }
            }
        }
        
        results = batch_results;
    } else {
        // Sequential execution (fallback)
        results = execute_sequential(operations, ctx, continue_on_error).await?;
    }

    let total_duration_ms = start.elapsed().as_millis() as u64;
    
    let successful = results.iter().filter(|r| r["success"].as_bool().unwrap_or(false)).count();
    let failed = results.len() - successful;

    Ok(json!({
        "total_operations": operations.len(),
        "successful": successful,
        "failed": failed,
        "parallel": parallel,
        "total_duration_ms": total_duration_ms,
        "results": results
    }))
}

// Helper function
async fn execute_single_operation(
    idx: usize,
    operation: Value,
    ctx: &ToolContext
) -> Result<Value> {
    let op_type = operation.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    
    let params = operation.get("params")
        .cloned()
        .unwrap_or(json!({}));

    let op_start = Instant::now();
    
    let (success, data, error) = match op_type {
        "read_file" => {
            match tool_read_file(params, ctx).await {
                Ok(result) => (true, Some(result), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        "get_symbols" => {
            match tool_get_symbols(params, ctx).await {
                Ok(result) => (true, Some(result), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        "search" => {
            match tool_search(params, ctx).await {
                Ok(result) => (true, Some(result), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        "skeleton" => {
            match tool_skeleton(params, ctx).await {
                Ok(result) => (true, Some(result), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        "get_references" => {
            match tool_get_references(params, ctx).await {
                Ok(result) => (true, Some(result), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        _ => (false, None, Some(format!("Unknown operation type: {}", op_type))),
    };

    let duration_ms = op_start.elapsed().as_millis() as u64;

    Ok(json!({
        "index": idx,
        "type": op_type,
        "success": success,
        "data": data,
        "error": error,
        "duration_ms": duration_ms
    }))
}
```

**Приоритет:** 🔥🔥🔥🔥 КРИТИЧНО (фича бесполезна без этого)  
**Время:** 6 часов (включая refactoring)  
**Файлы:** `src/daemon/tools.rs`, `src/daemon/state.rs`

---

#### ⚠️ Проблема 2: Поддерживает только 3 операции

**Текущий код** (lines 4593-4612):
```rust
let (success, data, error) = match op_type {
    "read_file" => {
        match tool_read_file(params.clone(), ctx).await {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        }
    }
    "get_symbols" => { /* ... */ }
    "search" => { /* ... */ }
    _ => (false, None, Some(format!("Unknown operation type: {}", op_type))),
};
```

**Проблема:** Отсутствуют важные операции:
- `skeleton`
- `get_references`
- `read_function_context`
- `read_types_only`

**Решение:** Добавить все операции (уже включено в код выше в `execute_single_operation`)

**Приоритет:** 🔥🔥 Высокий  
**Время:** 1 час (уже включен в решение выше)  

---

#### ⚠️ Проблема 3: Нет rate limiting

**Проблема:** Можно отправить batch с 1000 операций и убить сервер.

**Решение:** Добавить validation и rate limiting

```rust
// Config
const MAX_BATCH_SIZE: usize = 100;
const MAX_CONCURRENT: usize = 10;

async fn tool_batch_operations(args: Value, ctx: &ToolContext) -> Result<Value> {
    let operations = args.get("operations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| goferError::InvalidParams("operations array is required".into()))?;
    
    // Validate batch size
    if operations.len() > MAX_BATCH_SIZE {
        return Err(goferError::InvalidParams(
            format!("Too many operations. Max: {}, got: {}", MAX_BATCH_SIZE, operations.len())
        ).into());
    }
    
    // ... rest of code with semaphore limiting to MAX_CONCURRENT
}
```

**Приоритет:** 🔥🔥 Высокий  
**Время:** 0.5 часа (trivial)  
**Файлы:** `src/daemon/tools.rs`

---

### Feature 013: Итого

**Общее время исправления:** 7.5 часов (1 день)  
**Критичность:** КРИТИЧНАЯ - без parallel execution фича бесполезна

**Порядок выполнения:**
1. Refactor ToolContext + implement parallel (6 часов) - КРИТИЧНО
2. Add rate limiting (0.5 часа)

---

## Общий план исправления

### Приоритеты

#### 🔥🔥🔥🔥 Критичные задачи (должны быть сделаны первыми)

1. **batch_operations: реализовать parallel execution** - 6 часов
   - Без этого фича полностью бесполезна
   - Refactoring ToolContext затрагивает весь код

2. **read_function_context: реализовать type extraction** - 4 часа
   - Без этого экономия токенов недостаточна

3. **smart_file_selection: улучшить scoring** - 4 часа
   - Текущий scoring слишком примитивен для production

**Итого критичных:** 14 часов (≈2 рабочих дня)

---

#### 🔥🔥 Высокие приоритеты

4. **read_function_context: улучшить callees** - 3 часа
5. **read_function_context: import filtering** - 2 часа
6. **batch_operations: rate limiting** - 0.5 часа
7. **smart_file_selection: улучшить path scoring** - 3 часа
8. **read_types_only: doc comments + attributes** - 2 часа

**Итого высоких:** 10.5 часов (≈1.5 дня)

---

#### 🔥 Средние приоритеты

9. **smart_file_selection: query caching** - 1 час

**Итого средних:** 1 час

---

### Общее время

- **Критичные:** 14 часов (2 дня)
- **Высокие:** 10.5 часов (1.5 дня)
- **Средние:** 1 час (0.5 дня)

**ИТОГО: 25.5 часов (3-4 рабочих дня)**

---

## Рекомендуемый порядок выполнения

### День 1: batch_operations (критично)
**Цель:** Сделать фичу работающей

- [x] **09:00-12:00** (3 ч): Refactor ToolContext to Arc
  - Изменить `src/daemon/tools.rs`: `pub struct ToolContext<'a>` → `pub struct ToolContext`
  - Изменить все поля на `Arc<T>`
  - Implement `Clone` trait
  
- [x] **13:00-16:00** (3 ч): Implement parallel execution
  - Implement `execute_single_operation`
  - Add tokio::spawn with semaphore
  - Testing with small batches
  
- [x] **16:00-16:30** (0.5 ч): Add rate limiting
  - Validation for MAX_BATCH_SIZE
  - Update error messages

**Результат:** batch_operations полностью работает

---

### День 2: read_function_context
**Цель:** Довести до production quality

- [x] **09:00-13:00** (4 ч): Implement type extraction
  - Function `extract_referenced_types`
  - SQLite queries для type definitions
  - Integration в tool_read_function_context
  
- [x] **14:00-17:00** (3 ч): Improve callees extraction
  - Function `extract_callees`
  - AST-based call discovery
  - Extract callee code from same file

**Результат:** read_function_context экономит 90%+ токенов как задумано

---

### День 3: smart_file_selection + завершение read_function_context
**Цель:** Smart selection работает отлично

- [x] **09:00-13:00** (4 ч): Improve scoring algorithm
  - Adaptive weights
  - Recency boost
  - Size penalty
  - Confidence scoring
  
- [x] **14:00-16:00** (2 ч): Import filtering для read_function_context
  - Function `filter_used_imports`
  - Testing
  
- [x] **16:00-17:00** (1 ч): Query caching для smart_file_selection

**Результат:** Обе фичи production-ready

---

### День 4: Доделки + тестирование
**Цель:** Polish и качество

- [x] **09:00-12:00** (3 ч): Path scoring improvement
  - Edit distance
  - Stemming
  - Better directory weighting
  
- [x] **13:00-15:00** (2 ч): Doc comments для read_types_only
  - Tree-sitter based extraction
  - Attributes support
  
- [x] **15:00-17:00** (2 ч): Integration testing
  - Test all 4 фичи together
  - Performance benchmarks
  - Edge cases

**Результат:** Все фичи протестированы и готовы к релизу

---

## Критерии приемки (Acceptance Criteria)

### Feature 009: read_function_context

- [ ] Type extraction работает для Rust, TypeScript, Python
- [ ] Включает только используемые imports (не все)
- [ ] Callees extraction включает код функций (не только names)
- [ ] Token savings >= 90% vs полный read_file
- [ ] All tests pass

### Feature 010: read_types_only

- [ ] Doc comments extraction через tree-sitter
- [ ] Rust attributes включены (#[derive], etc.)
- [ ] Works для всех поддерживаемых языков
- [ ] Token savings >= 90%
- [ ] All tests pass

### Feature 011: smart_file_selection

- [ ] Adaptive scoring weights based on query type
- [ ] Recency boost работает
- [ ] Size penalty работает
- [ ] Confidence scoring included
- [ ] Path scoring uses edit distance
- [ ] Query results cached
- [ ] Top-3 accuracy >= 70% (manual testing)
- [ ] All tests pass

### Feature 013: batch_operations

- [ ] **Parallel execution работает** (КРИТИЧНО)
- [ ] Semaphore limiting to 10 concurrent
- [ ] Rate limiting (max 100 operations)
- [ ] Поддерживает все операции (read_file, skeleton, search, etc.)
- [ ] continue_on_error работает
- [ ] Latency reduction >= 3× vs sequential (benchmark)
- [ ] All tests pass

---

## Риски и митigation

### Риск 1: ToolContext refactoring breaks everything
**Вероятность:** Средняя  
**Влияние:** Критическое  
**Mitigation:**
- Делать в отдельной ветке
- Тщательное тестирование после refactoring
- Rollback plan готов

### Риск 2: Parallel execution memory issues
**Вероятность:** Низкая  
**Влияние:** Высокое  
**Mitigation:**
- Semaphore limiting to 10 concurrent
- Memory profiling с tokio-console
- Rate limiting на batch size

### Риск 3: Type extraction сложнее чем кажется
**Вероятность:** Средняя  
**Влияние:** Среднее  
**Mitigation:**
- Start с simple approach (SQLite lookup)
- Iterate если недостаточно
- Fallback: возвращать note вместо ошибки

---

## Метрики успеха

После завершения всех исправлений:

### Performance
- [ ] batch_operations: 3-5× latency reduction (benchmark)
- [ ] read_function_context: 90%+ token savings (test cases)
- [ ] read_types_only: 90%+ token savings (test cases)
- [ ] smart_file_selection: < 2s response time (benchmark)

### Quality
- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] No compiler warnings
- [ ] Code review approved

### Completeness
- [ ] Feature 009: 100% реализовано
- [ ] Feature 010: 100% реализовано
- [ ] Feature 011: 100% реализовано
- [ ] Feature 013: 100% реализовано

---

## Следующие шаги

1. **Review этого плана** с командой
2. **Создать задачи** в issue tracker
3. **Начать с Day 1** (batch_operations refactoring)
4. **Daily check-ins** для tracking прогресса

---

**Автор:** AI Code Audit  
**Дата:** 2026-02-16  
**Статус:** Ready for implementation
