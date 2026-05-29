# Feature: search_ranked - Smart Multi-Factor Ranking

**ID:** PHASE3-032  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 5 дней  
**Status:** Not Started  
**Phase:** 3 (Intelligence & Security - Smart Ranking)

---

## 📋 Описание

Умный поиск с multi-factor ranking. Учитывает не только semantic similarity, но и recency, stability, test coverage, code ownership, personal relevance.

### Проблема

**Simple semantic search:**
```
Query: "authentication"

Results (by similarity only):
1. old_auth.rs (deprecated, 95% match)
2. auth_backup.rs (unused, 90% match)
3. auth.rs (current, 85% match)

Problem: Устаревший код ranked выше!
```

**С smart ranking:**
```
Query: "authentication"

Results (multi-factor):
1. auth.rs (85% semantic, recently updated, stable, 90% coverage)
2. auth_v2.rs (80% semantic, your file, high activity)
3. old_auth.rs (95% semantic, BUT deprecated, low stability)

Smart: Актуальный код ranked выше!
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Multi-factor ranking с configurable weights
- ✅ Учитывать: similarity, recency, stability, coverage, ownership
- ✅ Personal workspace tracking
- ✅ 30-50% better relevance

### Non-Goals
- ❌ Не ML-based ranking (heuristic OK)
- ❌ Не automatic weight tuning

---

## 🏗️ Архитектура

```
┌─────────────────────────────────────────┐
│         search_ranked(query)            │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │ Vector Search   │
        │ (base results)  │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬────────────┬────────────┐
     │           │           │            │            │
┌────▼─────┐┌───▼────┐┌────▼──────┐┌────▼──────┐┌───▼────┐
│Semantic  ││Recency ││ Stability ││ Coverage  ││Personal│
│ Score    ││ Score  ││  Score    ││  Score    ││ Score  │
└──────────┘└────────┘└───────────┘└───────────┘└────────┘
                                │
                       ┌────────▼────────┐
                       │ Ranking Engine  │
                       │ (weighted sum)  │
                       └─────────────────┘
```

---

## 📊 Data Model

### MCP Tool Definition

```json
{
  "name": "search_ranked",
  "description": "Smart search с multi-factor ranking",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {"type": "string"},
      "context": {
        "type": "object",
        "properties": {
          "recent_changes": {"type": "boolean", "default": true},
          "test_coverage": {"type": "boolean", "default": true},
          "code_churn": {"type": "string", "enum": ["low", "medium", "high", "any"], "default": "any"},
          "my_workspace": {"type": "boolean", "default": false},
          "stability": {"type": "string", "enum": ["stable", "changing", "any"], "default": "any"}
        }
      },
      "limit": {"type": "number", "default": 10}
    },
    "required": ["query"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct RankedResult {
    pub file: String,
    pub line: u32,
    pub content: String,
    pub total_score: f32,
    pub score_breakdown: ScoreBreakdown,
    pub metadata: ResultMetadata,
}

#[derive(Serialize)]
pub struct ScoreBreakdown {
    pub semantic: f32,      // 0-1
    pub recency: f32,       // 0-1
    pub stability: f32,     // 0-1
    pub coverage: f32,      // 0-1
    pub ownership: f32,     // 0-1
    pub personal: f32,      // 0-1
}

#[derive(Serialize)]
pub struct ResultMetadata {
    pub last_modified: DateTime<Utc>,
    pub churn_rate: String,
    pub test_coverage_percent: f32,
    pub primary_owner: Option<String>,
    pub recently_accessed: bool,
}
```

---

## 💻 Implementation Details

### Ranking Engine

```rust
pub struct RankingEngine {
    weights: RankingWeights,
    workspace_tracker: WorkspaceTracker,
}

#[derive(Clone)]
pub struct RankingWeights {
    semantic: f32,     // 40%
    recency: f32,      // 20%
    stability: f32,    // 15%
    coverage: f32,     // 10%
    ownership: f32,    // 10%
    personal: f32,     // 5%
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            semantic: 0.40,
            recency: 0.20,
            stability: 0.15,
            coverage: 0.10,
            ownership: 0.10,
            personal: 0.05,
        }
    }
}

impl RankingEngine {
    pub async fn rank_results(
        &self,
        base_results: Vec<SearchResult>,
        context: RankingContext,
    ) -> Result<Vec<RankedResult>> {
        let mut ranked = Vec::new();
        
        for result in base_results {
            // Calculate individual scores
            let semantic_score = result.similarity;  // from vector search
            
            let recency_score = self.calculate_recency_score(&result).await?;
            let stability_score = self.calculate_stability_score(&result).await?;
            let coverage_score = self.calculate_coverage_score(&result).await?;
            let ownership_score = self.calculate_ownership_score(&result).await?;
            let personal_score = self.calculate_personal_score(&result).await?;
            
            // Weighted sum
            let total_score = 
                semantic_score * self.weights.semantic +
                recency_score * self.weights.recency +
                stability_score * self.weights.stability +
                coverage_score * self.weights.coverage +
                ownership_score * self.weights.ownership +
                personal_score * self.weights.personal;
            
            // Apply context filters
            if !self.matches_context(&result, &context).await? {
                continue;
            }
            
            ranked.push(RankedResult {
                file: result.file.clone(),
                line: result.line,
                content: result.content.clone(),
                total_score,
                score_breakdown: ScoreBreakdown {
                    semantic: semantic_score,
                    recency: recency_score,
                    stability: stability_score,
                    coverage: coverage_score,
                    ownership: ownership_score,
                    personal: personal_score,
                },
                metadata: self.gather_metadata(&result).await?,
            });
        }
        
        // Sort by total score
        ranked.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());
        
        Ok(ranked)
    }
    
    async fn calculate_recency_score(&self, result: &SearchResult) -> Result<f32> {
        // Get last modification time
        let last_modified = get_file_mtime(&result.file).await?;
        let age = Utc::now() - last_modified;
        
        // Score: 1.0 for < 7 days, decay to 0 over 1 year
        let days = age.num_days() as f32;
        let score = (1.0 - (days / 365.0)).max(0.0);
        
        Ok(score)
    }
    
    async fn calculate_stability_score(&self, result: &SearchResult) -> Result<f32> {
        // Get churn rate
        let churn = get_code_churn(&result.file, "3 months").await?;
        
        // Score: 1.0 for low churn, 0.5 for medium, 0.0 for high
        let score = match churn.commit_count {
            0..=5 => 1.0,
            6..=15 => 0.5,
            _ => 0.0,
        };
        
        Ok(score)
    }
    
    async fn calculate_coverage_score(&self, result: &SearchResult) -> Result<f32> {
        // Get test coverage for file
        let coverage = get_test_coverage(&result.file).await?;
        
        // Normalize to 0-1
        Ok(coverage.percent / 100.0)
    }
    
    async fn calculate_ownership_score(&self, result: &SearchResult) -> Result<f32> {
        // Check if current user is primary owner
        let owners = get_code_owners(&result.file).await?;
        
        if let Some(primary) = owners.first() {
            if primary.name == current_user() {
                return Ok(1.0);
            }
        }
        
        Ok(0.5)  // Default
    }
    
    async fn calculate_personal_score(&self, result: &SearchResult) -> Result<f32> {
        // Check workspace tracking
        let recent = self.workspace_tracker
            .is_recently_accessed(&result.file)
            .await?;
        
        if recent {
            Ok(1.0)
        } else {
            Ok(0.0)
        }
    }
    
    async fn matches_context(
        &self,
        result: &SearchResult,
        context: &RankingContext
    ) -> Result<bool> {
        // Filter by churn
        if context.code_churn != ChurnFilter::Any {
            let churn = get_code_churn(&result.file, "3 months").await?;
            // Apply filter...
        }
        
        // Filter by stability
        if context.stability != StabilityFilter::Any {
            // Check stability...
        }
        
        Ok(true)
    }
}
```

### Workspace Tracker

```rust
// src/intelligence/workspace_tracker.rs

pub struct WorkspaceTracker {
    db: SqlitePool,
}

impl WorkspaceTracker {
    pub async fn track_access(&self, file: &str) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO workspace_activity (file, accessed_at, access_count)
            VALUES (?, CURRENT_TIMESTAMP, 1)
            ON CONFLICT(file) DO UPDATE SET
                accessed_at = CURRENT_TIMESTAMP,
                access_count = access_count + 1
            "#,
            file
        )
        .execute(&self.db)
        .await?;
        
        Ok(())
    }
    
    pub async fn is_recently_accessed(&self, file: &str) -> Result<bool> {
        let recent = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) > 0 as is_recent
            FROM workspace_activity
            WHERE file = ?
              AND accessed_at > datetime('now', '-7 days')
            "#,
            file
        )
        .fetch_one(&self.db)
        .await?;
        
        Ok(recent != 0)
    }
}
```

---

## 📈 Success Metrics

### Relevance
- ✅ 30-50% better relevance vs simple search
- ✅ Top-3 accuracy > 80%

### Performance
- ⏱️ Response time < 1s
- 📊 Scoring overhead < 200ms

---

## 📚 Usage Example

```typescript
// Basic smart search
const results = await gofer.search_ranked({
  query: "authentication",
  context: {
    recent_changes: true,
    test_coverage: true,
    stability: "stable"
  }
});

results.forEach(r => {
  console.log(`${r.file} (score: ${r.total_score.toFixed(2)})`);
  console.log(`  Semantic: ${r.score_breakdown.semantic.toFixed(2)}`);
  console.log(`  Recency: ${r.score_breakdown.recency.toFixed(2)}`);
  console.log(`  Stability: ${r.score_breakdown.stability.toFixed(2)}`);
});
```

---

## ✅ Acceptance Criteria

- [ ] Multi-factor ranking implemented
- [ ] Configurable weights
- [ ] Workspace tracking works
- [ ] Context filters apply correctly
- [ ] 30%+ relevance improvement
- [ ] Response time < 1s
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** КРИТИЧЕСКИЙ - революционирует качество поиска!
