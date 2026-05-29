# Feature: error_recovery - Graceful Error Handling & Recovery

**ID:** PHASE0-016  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 0 (Foundation)

---

## 📋 Описание

Robust error handling, automatic recovery, и graceful degradation при failures. Ensures gofer MCP остается стабильным даже при partial failures.

### Проблема

**Current behavior (fail fast):**
```
Scenario: Embedding API down

AI: search("authentication")
gofer: ❌ CRASH - embedding API unreachable
→ Весь gofer MCP server down
→ AI не может работать вообще
```

**С error_recovery:**
```
AI: search("authentication")
gofer: ⚠️ Embedding API down, falling back to keyword search
→ Возвращает результаты (degraded mode)
→ AI продолжает работать
→ gofer автоматически retry embedding API в фоне
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Graceful degradation (fallback strategies)
- ✅ Automatic retry с exponential backoff
- ✅ Circuit breaker для external APIs
- ✅ Partial results вместо complete failure
- ✅ Error reporting + logging
- ✅ 99.9% uptime

### Non-Goals
- ❌ Не маскирует critical errors
- ❌ Не silent failures

---

## 🏗️ Архитектура

### Error Handling Layers

```
┌─────────────────────────────────────────┐
│         MCP Request                     │
└────────────────┬────────────────────────┘
                 │
        ┌────────▼────────┐
        │  Error Handler  │
        │   (top-level)   │
        └────────┬────────┘
                 │
     ┌───────────┼───────────┬────────────┐
     │           │           │            │
┌────▼─────┐ ┌──▼───┐ ┌────▼──────┐ ┌───▼────┐
│ Retry    │ │Circuit│ │ Fallback  │ │ Report │
│ Logic    │ │Breaker│ │ Strategy  │ │ Error  │
└──────────┘ └──────┘ └───────────┘ └────────┘
```

---

## 💻 Key Strategies

### 1. Graceful Degradation

```rust
// Search with fallback
pub async fn search_with_fallback(
    query: &str
) -> Result<SearchResults> {
    // Try vector search
    match vector_search(query).await {
        Ok(results) => Ok(results),
        Err(e) => {
            warn!("Vector search failed: {}, falling back to keyword", e);
            // Fallback to keyword search
            keyword_search(query).await
        }
    }
}
```

### 2. Circuit Breaker

```rust
pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitState>>,
    failure_threshold: usize,
    timeout: Duration,
}

enum CircuitState {
    Closed,        // Normal operation
    Open,          // Too many failures, reject requests
    HalfOpen,      // Testing if service recovered
}

impl CircuitBreaker {
    pub async fn call<F, T>(&self, f: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        let state = self.state.read().await;
        
        match *state {
            CircuitState::Open => {
                Err(anyhow!("Circuit breaker open"))
            }
            CircuitState::HalfOpen | CircuitState::Closed => {
                drop(state);
                
                match f.await {
                    Ok(result) => {
                        self.on_success().await;
                        Ok(result)
                    }
                    Err(e) => {
                        self.on_failure().await;
                        Err(e)
                    }
                }
            }
        }
    }
}
```

### 3. Retry Logic

```rust
pub async fn retry_with_backoff<F, T>(
    mut f: F,
    max_attempts: usize,
) -> Result<T>
where
    F: FnMut() -> BoxFuture<'static, Result<T>>,
{
    let mut attempt = 0;
    
    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                attempt += 1;
                
                if attempt >= max_attempts {
                    return Err(e);
                }
                
                let delay = Duration::from_millis(
                    100 * 2_u64.pow(attempt as u32)
                );
                
                warn!("Attempt {} failed: {}, retrying in {:?}", 
                      attempt, e, delay);
                
                tokio::time::sleep(delay).await;
            }
        }
    }
}
```

### 4. Partial Results

```rust
pub struct PartialSearchResults {
    pub results: Vec<SearchResult>,
    pub warnings: Vec<String>,
    pub degraded: bool,
}

// Return partial results instead of failing
pub async fn search_multi_source(
    query: &str
) -> Result<PartialSearchResults> {
    let mut results = Vec::new();
    let mut warnings = Vec::new();
    let mut degraded = false;
    
    // Try vector search
    match vector_search(query).await {
        Ok(vector_results) => results.extend(vector_results),
        Err(e) => {
            warnings.push(format!("Vector search failed: {}", e));
            degraded = true;
        }
    }
    
    // Try keyword search (always)
    match keyword_search(query).await {
        Ok(keyword_results) => results.extend(keyword_results),
        Err(e) => {
            warnings.push(format!("Keyword search failed: {}", e));
        }
    }
    
    if results.is_empty() {
        Err(anyhow!("All search methods failed"))
    } else {
        Ok(PartialSearchResults {
            results,
            warnings,
            degraded,
        })
    }
}
```

---

## 📊 Error Categories

### Recoverable Errors (retry)
- Network timeouts
- Temporary API failures
- Rate limiting
- Lock contention

### Degradable Errors (fallback)
- Embedding API down → keyword search
- Vector DB down → SQL-only search
- Cache miss → direct DB query

### Fatal Errors (fail fast)
- Database corruption
- Out of memory
- Invalid configuration
- Security violations

---

## 🧪 Testing

```rust
#[tokio::test]
async fn test_circuit_breaker_opens_on_failures() {
    let breaker = CircuitBreaker::new(3, Duration::from_secs(60));
    
    // Simulate 3 failures
    for _ in 0..3 {
        let result = breaker.call(async { 
            Err(anyhow!("Simulated failure")) 
        }).await;
        assert!(result.is_err());
    }
    
    // Circuit should be open now
    let state = breaker.state.read().await;
    assert!(matches!(*state, CircuitState::Open));
    
    // Further calls should fail immediately
    let result = breaker.call(async { Ok(()) }).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().to_string(), "Circuit breaker open");
}

#[tokio::test]
async fn test_graceful_degradation() {
    let search = SearchService::new();
    
    // Simulate vector search failure
    search.vector_db.set_unavailable(true);
    
    // Should fall back to keyword search
    let results = search.search("test query").await.unwrap();
    
    assert!(!results.results.is_empty());
    assert!(results.degraded);
    assert!(!results.warnings.is_empty());
}

#[tokio::test]
async fn test_retry_with_backoff() {
    let mut attempt = 0;
    
    let result = retry_with_backoff(
        || {
            Box::pin(async {
                attempt += 1;
                if attempt < 3 {
                    Err(anyhow!("Temporary failure"))
                } else {
                    Ok("Success")
                }
            })
        },
        5
    ).await;
    
    assert!(result.is_ok());
    assert_eq!(attempt, 3);
}
```

---

## 📈 Success Metrics

### Availability
- ✅ 99.9% uptime
- ✅ < 0.1% complete failures
- ✅ Graceful degradation в 95%+ failure scenarios

### Recovery
- ⏱️ Automatic recovery < 60 seconds
- 🔄 Successful retry rate > 80%
- ⚡ Circuit breaker trip time < 10s

### User Experience
- ✅ Partial results > no results
- ✅ Clear error messages
- ✅ Degraded mode indicators

---

## 📚 Error Response Format

```json
{
  "success": false,
  "error": {
    "code": "VECTOR_SEARCH_UNAVAILABLE",
    "message": "Vector search temporarily unavailable, using keyword search",
    "category": "degraded",
    "retry_after": 60,
    "suggestions": [
      "Results may be less relevant than usual",
      "Vector search will be retried automatically"
    ]
  },
  "partial_results": {
    "data": [...],
    "degraded": true
  }
}
```

---

## ✅ Acceptance Criteria

- [ ] Circuit breaker prevents cascading failures
- [ ] Automatic retry with exponential backoff
- [ ] Graceful degradation for all critical paths
- [ ] Partial results instead of complete failure
- [ ] Clear error messages with actionable suggestions
- [ ] 99.9% uptime in tests
- [ ] All error scenarios tested
- [ ] Recovery time < 60s

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16  
**Assigned To:** TBD

**Impact:** КРИТИЧЕСКИЙ - без этого gofer MCP нестабилен в production.
