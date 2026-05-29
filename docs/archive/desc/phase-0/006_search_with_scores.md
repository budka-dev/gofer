# Feature: search_with_scores - Ранжированный поиск

**ID:** PHASE0-006  
**Priority:** 🔥🔥🔥 High  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 0 (Quick Wins)  
**Depends On:** None (extends existing search)

---

## 📋 Описание

Улучшение существующего `search()` tool с добавлением relevance scores, match reasons и preview mode. Позволяет AI делать более informed decisions о том, какие файлы читать полностью. Экономит 80% токенов при широких поисках.

### Проблема

**Текущий search():**
```typescript
const results = await gofer.search({ query: "authentication", limit: 20 });

// Returns:
// - 20 full content snippets (200+ lines each)
// - No indication which results are most relevant
// - No context about WHY they matched
// - AI must read all 4000 lines to find best match
// - 10000+ tokens wasted
```

**Проблемы:**
- Нет scores → все результаты выглядят одинаково важными
- Нет reason → не понятно почему matched (function name? doc? code?)
- Полный content → слишком много токенов
- AI читает много ненужного контекста

### Решение

**Enhanced search with scores:**
```typescript
const results = await gofer.search({
  query: "authentication",
  limit: 20,
  include_scores: true
});

// Returns:
// - Relevance score (0.0-1.0) for each result
// - Match reason ("function name", "doc comment", "code content")
// - Preview (first 2-3 lines) instead of full content
// - AI reads TOP 3 in detail, skips rest
// - 8000 tokens saved (80% reduction)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Add relevance scores to all search results
- ✅ Provide match reasons for better understanding
- ✅ Implement preview mode (2-3 lines vs full content)
- ✅ Enable score-based filtering (e.g., > 0.7 only)
- ✅ 80% token savings for wide searches

### Non-Goals
- ❌ Not changing search algorithm (still using LanceDB vector search)
- ❌ Not adding new search types (semantic, code, etc.)
- ❌ Not replacing existing search (just enhancing)

---

## 🏗️ Архитектура

### Components

```
┌─────────────────────────────────────────┐
│         MCP Tool Handler                │
│     search() [enhanced]                 │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Search Engine  │
        │   (existing)    │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┐
     │           │           │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐
│LanceDB   │ │Score │ │ Preview   │
│Vector    │ │Norm. │ │ Generator │
│Search    │ │      │ │           │
└──────────┘ └──────┘ └───────────┘
```

### Search Flow (Enhanced)

```
1. Parse query → 2. Vector search → 3. Get scores → 4. Classify matches
   ↓               ↓                  ↓              ↓
Query text      LanceDB returns    Normalize      Determine WHY
                vectors + raw      0.0-1.0        matched
                scores             
                                   ↓
                5. Generate preview → 6. Format response
                   ↓                     ↓
                Truncate content      Return ranked
                to 2-3 lines          results
```

---

## 🔧 API Specification

### Enhanced search() Tool

**Backward compatible:** Old calls still work, new parameters are optional.

```json
{
  "name": "search",
  "description": "Semantic search with optional relevance scores and preview mode",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query"
      },
      "limit": {
        "type": "integer",
        "default": 10,
        "description": "Maximum results"
      },
      "include_scores": {
        "type": "boolean",
        "default": false,
        "description": "Include relevance scores (0.0-1.0)"
      },
      "preview_mode": {
        "type": "boolean",
        "default": false,
        "description": "Return short preview instead of full content"
      },
      "min_score": {
        "type": "number",
        "default": 0.0,
        "description": "Minimum relevance score (filters low-quality results)"
      },
      "include_context": {
        "type": "boolean",
        "default": true,
        "description": "Include context (function/class name where match found)"
      }
    },
    "required": ["query"]
  }
}
```

### Response Schema (Enhanced)

```rust
#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub total_results: usize,
    pub results: Vec<SearchResult>,
    pub search_time_ms: u64,
}

#[derive(Serialize)]
pub struct SearchResult {
    pub file: String,
    pub line: u32,
    pub content: String,  // Full content OR preview (based on preview_mode)
    
    // NEW FIELDS
    pub score: Option<f32>,  // 0.0 - 1.0 (if include_scores=true)
    pub match_reason: Option<MatchReason>,
    pub context: Option<String>,  // Function/class name where match found
    pub preview: Option<String>,  // First 2-3 lines (if preview_mode=true)
}

#[derive(Serialize)]
pub enum MatchReason {
    FunctionName,      // Query matches function/method name
    ClassName,         // Query matches class/struct name
    DocComment,        // Query matches documentation
    CodeContent,       // Query matches code body
    ImportStatement,   // Query matches import/use statement
    TypeDefinition,    // Query matches type definition
    Mixed,             // Multiple reasons
}
```

### Example Response (with scores & preview)

```json
{
  "query": "authentication",
  "total_results": 15,
  "results": [
    {
      "file": "src/auth/verify.rs",
      "line": 45,
      "content": "pub async fn verify_authentication(token: &str) -> Result<Claims> {\n    // Verify JWT token\n    ...",
      "score": 0.95,
      "match_reason": "FunctionName",
      "context": "verify_authentication",
      "preview": "pub async fn verify_authentication(token: &str) -> Result<Claims> {\n    // Verify JWT token"
    },
    {
      "file": "src/auth/middleware.rs",
      "line": 12,
      "content": "/// Authentication middleware for HTTP requests\npub struct AuthMiddleware { ... }",
      "score": 0.87,
      "match_reason": "DocComment",
      "context": "AuthMiddleware",
      "preview": "/// Authentication middleware for HTTP requests\npub struct AuthMiddleware {"
    },
    {
      "file": "src/config.rs",
      "line": 89,
      "content": "// Authentication configuration\npub struct AuthConfig { ... }",
      "score": 0.65,
      "match_reason": "CodeContent",
      "context": "AuthConfig",
      "preview": "// Authentication configuration\npub struct AuthConfig {"
    }
  ],
  "search_time_ms": 45
}
```

### Companion Tool: search_preview (Shortcut)

New tool for common use case:

```json
{
  "name": "search_preview",
  "description": "Fast search with previews and scores (shortcut for search with preview_mode=true)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "limit": { "type": "integer", "default": 20 },
      "min_score": { "type": "number", "default": 0.5 }
    },
    "required": ["query"]
  }
}
```

Equivalent to:
```typescript
search({
  query,
  limit,
  min_score,
  include_scores: true,
  preview_mode: true,
  include_context: true
})
```

---

## 💻 Implementation Details

### Score Normalization

LanceDB returns raw distance scores. Need to normalize to 0.0-1.0:

```rust
// src/search/scorer.rs

pub struct ScoreNormalizer {
    // Calibration data from historical searches
    min_distance: f32,
    max_distance: f32,
}

impl ScoreNormalizer {
    pub fn new() -> Self {
        // Typical LanceDB cosine distance range: 0.0 (identical) to 2.0 (opposite)
        Self {
            min_distance: 0.0,
            max_distance: 2.0,
        }
    }
    
    pub fn normalize(&self, raw_distance: f32) -> f32 {
        // Convert distance to similarity score
        // Lower distance = higher similarity
        // Map [0.0, 2.0] → [1.0, 0.0]
        
        let clamped = raw_distance.clamp(self.min_distance, self.max_distance);
        let normalized = 1.0 - (clamped / self.max_distance);
        
        // Apply sigmoid for better distribution
        self.apply_sigmoid(normalized)
    }
    
    fn apply_sigmoid(&self, score: f32) -> f32 {
        // Sigmoid: spreads scores for better differentiation
        // Makes top results clearly better than mediocre ones
        
        // Steepness parameter (higher = more aggressive)
        let k = 10.0;
        let midpoint = 0.5;
        
        1.0 / (1.0 + f32::exp(-k * (score - midpoint)))
    }
}
```

### Match Reason Classification

```rust
// src/search/classifier.rs

pub struct MatchReasonClassifier {
    sqlite: SqliteStorage,
}

impl MatchReasonClassifier {
    pub async fn classify(
        &self,
        query: &str,
        result: &RawSearchResult,
    ) -> Result<MatchReason> {
        let query_lower = query.to_lowercase();
        let content_lower = result.content.to_lowercase();
        
        // Check various patterns
        let mut reasons = Vec::new();
        
        // 1. Function/method name
        if self.matches_function_name(&query_lower, result).await? {
            reasons.push(MatchReason::FunctionName);
        }
        
        // 2. Class/struct name
        if self.matches_class_name(&query_lower, result).await? {
            reasons.push(MatchReason::ClassName);
        }
        
        // 3. Doc comment
        if self.matches_doc_comment(&query_lower, &content_lower) {
            reasons.push(MatchReason::DocComment);
        }
        
        // 4. Import statement
        if self.matches_import(&query_lower, &content_lower) {
            reasons.push(MatchReason::ImportStatement);
        }
        
        // 5. Type definition
        if self.matches_type_definition(&query_lower, result).await? {
            reasons.push(MatchReason::TypeDefinition);
        }
        
        // 6. Code content (fallback)
        if reasons.is_empty() {
            reasons.push(MatchReason::CodeContent);
        }
        
        // Return primary reason or Mixed
        Ok(if reasons.len() == 1 {
            reasons[0].clone()
        } else {
            MatchReason::Mixed
        })
    }
    
    async fn matches_function_name(
        &self,
        query: &str,
        result: &RawSearchResult,
    ) -> Result<bool> {
        // Query symbols table
        let symbols = sqlx::query!(
            r#"
            SELECT name
            FROM symbols
            WHERE file_id = (SELECT id FROM files WHERE path = ?)
              AND line <= ? AND line >= ? - 10
              AND kind = 'function'
            "#,
            result.file,
            result.line,
            result.line
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        // Check if query matches any function name
        Ok(symbols.iter().any(|s| {
            s.name.to_lowercase().contains(query) ||
            query.contains(&s.name.to_lowercase())
        }))
    }
    
    async fn matches_class_name(
        &self,
        query: &str,
        result: &RawSearchResult,
    ) -> Result<bool> {
        let symbols = sqlx::query!(
            r#"
            SELECT name
            FROM symbols
            WHERE file_id = (SELECT id FROM files WHERE path = ?)
              AND line <= ? AND line >= ? - 5
              AND kind IN ('struct', 'class', 'enum', 'interface')
            "#,
            result.file,
            result.line,
            result.line
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        Ok(symbols.iter().any(|s| {
            s.name.to_lowercase().contains(query) ||
            query.contains(&s.name.to_lowercase())
        }))
    }
    
    fn matches_doc_comment(&self, query: &str, content: &str) -> bool {
        // Check if match is in doc comment
        let lines: Vec<&str> = content.lines().collect();
        
        for line in lines {
            let trimmed = line.trim();
            if (trimmed.starts_with("///") || 
                trimmed.starts_with("/**") ||
                trimmed.starts_with("//!") ||
                trimmed.starts_with("#")) &&
               trimmed.to_lowercase().contains(query)
            {
                return true;
            }
        }
        
        false
    }
    
    fn matches_import(&self, query: &str, content: &str) -> bool {
        let first_line = content.lines().next().unwrap_or("");
        let trimmed = first_line.trim();
        
        (trimmed.starts_with("use ") ||
         trimmed.starts_with("import ") ||
         trimmed.starts_with("from ")) &&
        trimmed.to_lowercase().contains(query)
    }
    
    async fn matches_type_definition(
        &self,
        query: &str,
        result: &RawSearchResult,
    ) -> Result<bool> {
        // Similar to class name, but broader
        let symbols = sqlx::query!(
            r#"
            SELECT name
            FROM symbols
            WHERE file_id = (SELECT id FROM files WHERE path = ?)
              AND line = ?
              AND kind IN ('type', 'typedef', 'alias')
            "#,
            result.file,
            result.line
        )
        .fetch_all(&self.sqlite.pool)
        .await?;
        
        Ok(symbols.iter().any(|s| {
            s.name.to_lowercase().contains(query)
        }))
    }
}
```

### Preview Generator

```rust
// src/search/preview.rs

pub struct PreviewGenerator;

impl PreviewGenerator {
    pub fn generate(content: &str, max_lines: usize) -> String {
        let lines: Vec<&str> = content.lines().take(max_lines).collect();
        
        let mut preview = lines.join("\n");
        
        // Add truncation indicator if needed
        if content.lines().count() > max_lines {
            preview.push_str("\n...");
        }
        
        preview
    }
    
    pub fn generate_smart(content: &str, query: &str) -> String {
        // Smart preview: show lines around match
        let lines: Vec<&str> = content.lines().collect();
        let query_lower = query.to_lowercase();
        
        // Find first line containing query
        let match_line_idx = lines.iter()
            .position(|line| line.to_lowercase().contains(&query_lower));
        
        if let Some(idx) = match_line_idx {
            // Show: 1 line before + match line + 1 line after
            let start = idx.saturating_sub(1);
            let end = (idx + 2).min(lines.len());
            
            let preview_lines = &lines[start..end];
            let mut preview = preview_lines.join("\n");
            
            if start > 0 {
                preview = format!("...\n{}", preview);
            }
            if end < lines.len() {
                preview.push_str("\n...");
            }
            
            preview
        } else {
            // Fallback: first 3 lines
            Self::generate(content, 3)
        }
    }
}
```

### Context Extraction

```rust
// src/search/context.rs

pub struct ContextExtractor {
    sqlite: SqliteStorage,
}

impl ContextExtractor {
    pub async fn extract_context(
        &self,
        file: &str,
        line: u32,
    ) -> Result<Option<String>> {
        // Find nearest symbol (function/class) containing this line
        let symbol = sqlx::query!(
            r#"
            SELECT name, kind
            FROM symbols
            WHERE file_id = (SELECT id FROM files WHERE path = ?)
              AND line <= ?
            ORDER BY line DESC
            LIMIT 1
            "#,
            file,
            line
        )
        .fetch_optional(&self.sqlite.pool)
        .await?;
        
        Ok(symbol.map(|s| {
            match s.kind.as_str() {
                "function" => format!("fn {}", s.name),
                "struct" => format!("struct {}", s.name),
                "class" => format!("class {}", s.name),
                "enum" => format!("enum {}", s.name),
                _ => s.name,
            }
        }))
    }
}
```

### Enhanced Search Handler

```rust
// src/daemon/tools/search.rs

pub async fn handle_search_enhanced(
    args: &Map<String, Value>,
    sqlite: &SqliteStorage,
    lance: &LanceStorage,
) -> Result<Value> {
    let query = args.get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing query"))?;
    
    let limit = args.get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;
    
    let include_scores = args.get("include_scores")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    let preview_mode = args.get("preview_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    let min_score = args.get("min_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    
    let include_context = args.get("include_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    
    let start = Instant::now();
    
    // Perform vector search
    let raw_results = lance.search(query, limit * 2).await?;  // Fetch extra for filtering
    
    // Initialize helpers
    let scorer = ScoreNormalizer::new();
    let classifier = MatchReasonClassifier::new(sqlite.clone());
    let context_extractor = ContextExtractor::new(sqlite.clone());
    
    // Process results
    let mut results = Vec::new();
    
    for raw in raw_results {
        // Normalize score
        let score = scorer.normalize(raw.distance);
        
        // Filter by min_score
        if score < min_score {
            continue;
        }
        
        // Classify match reason
        let match_reason = if include_scores {
            Some(classifier.classify(query, &raw).await?)
        } else {
            None
        };
        
        // Extract context
        let context = if include_context {
            context_extractor.extract_context(&raw.file, raw.line).await?
        } else {
            None
        };
        
        // Generate preview if requested
        let (content, preview) = if preview_mode {
            let preview = PreviewGenerator::generate_smart(&raw.content, query);
            (preview.clone(), Some(preview))
        } else {
            (raw.content.clone(), None)
        };
        
        results.push(SearchResult {
            file: raw.file,
            line: raw.line,
            content,
            score: if include_scores { Some(score) } else { None },
            match_reason,
            context,
            preview,
        });
        
        // Stop if reached desired limit
        if results.len() >= limit {
            break;
        }
    }
    
    let search_time_ms = start.elapsed().as_millis() as u64;
    
    let response = SearchResponse {
        query: query.to_string(),
        total_results: results.len(),
        results,
        search_time_ms,
    };
    
    Ok(serde_json::to_value(response)?)
}
```

---

## 🧪 Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_score_normalization() {
    let scorer = ScoreNormalizer::new();
    
    // Test various distances
    assert!(scorer.normalize(0.0) > 0.9);   // Very close
    assert!(scorer.normalize(0.5) > 0.7);   // Close
    assert!(scorer.normalize(1.0) < 0.6);   // Medium
    assert!(scorer.normalize(1.5) < 0.3);   // Far
    assert!(scorer.normalize(2.0) < 0.1);   // Very far
}

#[tokio::test]
async fn test_match_reason_classification() {
    let (sqlite, _) = setup_test_db().await;
    let classifier = MatchReasonClassifier::new(sqlite);
    
    // Function name match
    let result = RawSearchResult {
        file: "test.rs".into(),
        line: 10,
        content: "pub fn authenticate(user: &str) -> bool { ... }".into(),
        distance: 0.1,
    };
    
    let reason = classifier.classify("authenticate", &result).await.unwrap();
    assert!(matches!(reason, MatchReason::FunctionName));
    
    // Doc comment match
    let result2 = RawSearchResult {
        file: "test.rs".into(),
        line: 5,
        content: "/// Authenticate user credentials\npub fn check() { ... }".into(),
        distance: 0.2,
    };
    
    let reason2 = classifier.classify("authenticate", &result2).await.unwrap();
    assert!(matches!(reason2, MatchReason::DocComment));
}

#[tokio::test]
async fn test_preview_generation() {
    let content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
    
    let preview = PreviewGenerator::generate(content, 3);
    assert_eq!(preview.lines().count(), 4);  // 3 lines + "..."
    assert!(preview.ends_with("..."));
    
    let smart = PreviewGenerator::generate_smart(content, "Line 3");
    assert!(smart.contains("Line 2"));
    assert!(smart.contains("Line 3"));
    assert!(smart.contains("Line 4"));
}

#[tokio::test]
async fn test_search_with_scores() {
    let (sqlite, lance) = setup_test_environment().await;
    
    // Index test data
    index_test_files(&sqlite, &lance).await;
    
    let mut args = Map::new();
    args.insert("query".into(), Value::String("authentication".into()));
    args.insert("include_scores".into(), Value::Bool(true));
    args.insert("limit".into(), Value::Number(10.into()));
    
    let response = handle_search_enhanced(&args, &sqlite, &lance)
        .await
        .unwrap();
    
    let resp: SearchResponse = serde_json::from_value(response).unwrap();
    
    assert!(resp.results.len() > 0);
    
    // Check that results have scores
    assert!(resp.results[0].score.is_some());
    
    // Check that scores are sorted descending
    let scores: Vec<f32> = resp.results.iter()
        .filter_map(|r| r.score)
        .collect();
    
    for i in 1..scores.len() {
        assert!(scores[i-1] >= scores[i], "Results should be sorted by score");
    }
}

#[tokio::test]
async fn test_search_preview_mode() {
    let (sqlite, lance) = setup_test_environment().await;
    
    let mut args = Map::new();
    args.insert("query".into(), Value::String("test".into()));
    args.insert("preview_mode".into(), Value::Bool(true));
    
    let response = handle_search_enhanced(&args, &sqlite, &lance)
        .await
        .unwrap();
    
    let resp: SearchResponse = serde_json::from_value(response).unwrap();
    
    // Check that content is truncated
    for result in resp.results {
        assert!(result.content.lines().count() <= 4);  // 3 lines + "..."
        assert!(result.preview.is_some());
    }
}

#[tokio::test]
async fn test_min_score_filtering() {
    let (sqlite, lance) = setup_test_environment().await;
    
    let mut args = Map::new();
    args.insert("query".into(), Value::String("random_query".into()));
    args.insert("include_scores".into(), Value::Bool(true));
    args.insert("min_score".into(), Value::Number(0.7.into()));
    
    let response = handle_search_enhanced(&args, &sqlite, &lance)
        .await
        .unwrap();
    
    let resp: SearchResponse = serde_json::from_value(response).unwrap();
    
    // All results should have score >= 0.7
    for result in resp.results {
        if let Some(score) = result.score {
            assert!(score >= 0.7, "Score {} below minimum 0.7", score);
        }
    }
}
```

---

## 📈 Success Metrics

### Token Savings
- **Target:** 80% reduction for wide searches (limit=20+)
- **Measurement:** Compare preview mode vs full content

**Example:**
- Full mode: 20 results × 200 lines = 4000 lines = 10000 tokens
- Preview mode: 20 results × 3 lines = 60 lines = 200 tokens
- **Savings:** 98% 🎉

### Score Quality
- Top result should be "obviously best" (score > 0.8)
- Clear separation between good/mediocre results
- No score ties (sigmoid helps)

### Performance
- ⏱️ Score calculation: < 10ms overhead
- ⏱️ Preview generation: < 5ms overhead
- ⏱️ Total: < 50ms vs existing search

---

## 📚 Usage Examples

### Example 1: High-Confidence Results Only

```typescript
// Only show results we're confident about
const results = await gofer.search({
  query: "authentication",
  include_scores: true,
  min_score: 0.8  // 80%+ confidence
});

// Read only high-confidence matches in detail
for (const result of results.results) {
  const full = await gofer.read_file({ file_path: result.file });
  // Analyze...
}
```

### Example 2: Quick Overview with Previews

```typescript
// Fast overview of all matches
const results = await gofer.search({
  query: "error handling",
  limit: 30,
  preview_mode: true,
  include_scores: true
});

// Sort by score, show top 5
const top5 = results.results
  .sort((a, b) => b.score - a.score)
  .slice(0, 5);

console.log("Top 5 matches:");
top5.forEach(r => {
  console.log(`${r.score.toFixed(2)} - ${r.file}:${r.line}`);
  console.log(r.preview);
});
```

### Example 3: Understand Match Reasons

```typescript
const results = await gofer.search({
  query: "database",
  include_scores: true
});

// Group by match reason
const byReason = {};
for (const result of results.results) {
  const reason = result.match_reason || "Unknown";
  byReason[reason] = (byReason[reason] || 0) + 1;
}

console.log("Matches by type:");
console.log(byReason);
// {
//   FunctionName: 5,
//   DocComment: 12,
//   CodeContent: 8
// }
```

---

## ✅ Acceptance Criteria

- [ ] Scores added to search results
- [ ] Scores are normalized 0.0-1.0
- [ ] Top results clearly better than mediocre (sigmoid working)
- [ ] Match reasons classified correctly (>80% accuracy)
- [ ] Preview mode generates 2-3 line summaries
- [ ] Context extraction works for functions/classes
- [ ] min_score filtering works
- [ ] Backward compatible (old calls still work)
- [ ] Performance: < 50ms overhead vs existing search
- [ ] All unit tests pass
- [ ] Documentation complete

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Note:** This enhancement dramatically improves search usability. AI can make smart decisions about which results to explore in depth based on scores and previews!
