# Feature: get_function_metrics - Performance Metrics

**ID:** PHASE2-027  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 2 (Production Observability)

---

## 📋 Описание

Интеграция с Prometheus для получения production метрик функций: latency (p50/p95/p99), throughput, error rate.

### Проблема

```
AI: "Эта функция медленная?"
→ Нет visibility в production performance

Developer: "Ухудшилась ли latency после изменений?"
→ Нет baseline для сравнения
```

### Решение

```typescript
const metrics = await gofer.get_function_metrics({
  function: "process_payment",
  time_range: "24h"
});

// Returns:
// Latency: p50=45ms, p95=120ms, p99=450ms
// Throughput: 1,234 calls/sec
// Error rate: 0.5%
// vs Baseline: +15% latency (regression!)
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Prometheus integration (PromQL)
- ✅ Latency percentiles (p50, p95, p99)
- ✅ Throughput metrics
- ✅ Error rate tracking
- ✅ Baseline comparison

### Non-Goals
- ❌ Не создает metrics (use instrumentation)
- ❌ Не alerting

---

## 🔧 API Specification

```json
{
  "name": "get_function_metrics",
  "description": "Получить production метрики функции",
  "inputSchema": {
    "type": "object",
    "properties": {
      "function": {"type": "string"},
      "time_range": {"type": "string", "default": "24h"}
    },
    "required": ["function"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct FunctionMetrics {
    pub function: String,
    pub latency: LatencyMetrics,
    pub throughput: ThroughputMetrics,
    pub error_rate: f32,
    pub baseline_comparison: Option<BaselineComparison>,
}

#[derive(Serialize)]
pub struct LatencyMetrics {
    pub p50: f32,  // milliseconds
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
}

#[derive(Serialize)]
pub struct ThroughputMetrics {
    pub calls_per_second: f32,
    pub total_calls: usize,
}

#[derive(Serialize)]
pub struct BaselineComparison {
    pub latency_change_percent: f32,
    pub throughput_change_percent: f32,
    pub regression: bool,
}
```

---

## 💻 Implementation

```rust
pub async fn get_function_metrics(
    function: &str,
    time_range: &str
) -> Result<FunctionMetrics> {
    let prom_client = PrometheusClient::new()?;
    
    // 1. Query latency histogram
    let latency_query = format!(
        "histogram_quantile(0.50, sum(rate(function_duration_seconds_bucket{{function=\"{}\"}}[{}])) by (le))",
        function, time_range
    );
    
    let p50 = prom_client.query(&latency_query).await?;
    
    // Similar for p95, p99
    let p95 = prom_client.query(&latency_query.replace("0.50", "0.95")).await?;
    let p99 = prom_client.query(&latency_query.replace("0.50", "0.99")).await?;
    
    // 2. Query throughput
    let throughput_query = format!(
        "rate(function_calls_total{{function=\"{}\"}}[{}])",
        function, time_range
    );
    
    let throughput = prom_client.query(&throughput_query).await?;
    
    // 3. Query error rate
    let error_query = format!(
        "rate(function_errors_total{{function=\"{}\"}}[{}]) / rate(function_calls_total{{function=\"{}\"}}[{}])",
        function, time_range, function, time_range
    );
    
    let error_rate = prom_client.query(&error_query).await?;
    
    // 4. Baseline comparison
    let baseline = get_baseline_metrics(function).await?;
    let comparison = compare_with_baseline(&metrics, &baseline);
    
    Ok(FunctionMetrics {
        function: function.to_string(),
        latency: LatencyMetrics {
            p50: p50 * 1000.0, // convert to ms
            p95: p95 * 1000.0,
            p99: p99 * 1000.0,
            max: 0.0, // TODO
        },
        throughput: ThroughputMetrics {
            calls_per_second: throughput,
            total_calls: 0, // TODO
        },
        error_rate: error_rate * 100.0,
        baseline_comparison: Some(comparison),
    })
}

async fn get_baseline_metrics(function: &str) -> Result<FunctionMetrics> {
    // Query historical baseline (7d average)
    // Store in cache or database
    todo!()
}

fn compare_with_baseline(
    current: &FunctionMetrics,
    baseline: &FunctionMetrics
) -> BaselineComparison {
    let latency_change = ((current.latency.p95 - baseline.latency.p95) / baseline.latency.p95) * 100.0;
    let throughput_change = ((current.throughput.calls_per_second - baseline.throughput.calls_per_second) / baseline.throughput.calls_per_second) * 100.0;
    
    BaselineComparison {
        latency_change_percent: latency_change,
        throughput_change_percent: throughput_change,
        regression: latency_change > 10.0 || throughput_change < -10.0,
    }
}
```

---

## 📈 Success Metrics

- ✅ Accurate metrics from Prometheus
- ✅ Baseline comparison detects regressions
- ⏱️ Response time < 2s

---

## ✅ Acceptance Criteria

- [ ] Prometheus integration works
- [ ] Latency percentiles accurate
- [ ] Throughput calculated correctly
- [ ] Baseline comparison implemented
- [ ] Regression detection works
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
