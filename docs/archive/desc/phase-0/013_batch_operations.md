# Feature: batch_operations - Пакетные операции

**ID:** PHASE0-013  
**Priority:** 🔥🔥🔥 High  
**Effort:** 2 дня  
**Status:** Not Started  
**Phase:** 0 (Performance)

---

## 📋 Описание

MCP tool для выполнения множественных операций чтения/поиска за один запрос. Снижает latency и количество round-trips между AI и gofer MCP server.

### Проблема

**Сценарий: анализ нескольких модулей**

```
AI хочет проанализировать auth систему:

Without batch:
1. read_file("auth/mod.rs")     → 200ms
2. read_file("auth/jwt.rs")     → 200ms  
3. read_file("auth/session.rs") → 200ms
4. get_symbols("auth/mod.rs")   → 150ms
5. get_symbols("auth/jwt.rs")   → 150ms

Total: 5 requests, 900ms latency
Network overhead: 5× protocol handshakes
```

**С batch_operations:**
```
AI: batch_operations([
  {read_file: "auth/mod.rs"},
  {read_file: "auth/jwt.rs"},
  {read_file: "auth/session.rs"},
  {get_symbols: "auth/mod.rs"},
  {get_symbols: "auth/jwt.rs"}
])

Total: 1 request, 250ms latency
Network overhead: 1× protocol handshake
Speedup: 3.6× быстрее!
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Batch multiple read/search operations
- ✅ 3-5× latency reduction
- ✅ Поддержка: read_file, get_symbols, search, get_references
- ✅ Параллельное выполнение операций
- ✅ Partial success (если одна операция fails, другие продолжаются)

### Non-Goals
- ❌ Не поддерживает write операции
- ❌ Не транзакционный (операции независимы)
- ❌ Не гарантирует порядок выполнения

---

## 🔧 API Specification

```json
{
  "name": "batch_operations",
  "description": "Выполнить множественные операции чтения за один запрос. Снижает latency в 3-5×.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "operations": {
        "type": "array",
        "description": "Список операций для выполнения",
        "items": {
          "type": "object",
          "properties": {
            "type": {
              "type": "string",
              "enum": ["read_file", "get_symbols", "search", "get_references", "skeleton"]
            },
            "params": {"type": "object"}
          }
        }
      },
      "parallel": {
        "type": "boolean",
        "default": true,
        "description": "Выполнять операции параллельно"
      },
      "continue_on_error": {
        "type": "boolean",
        "default": true,
        "description": "Продолжать если одна операция fails"
      }
    },
    "required": ["operations"]
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct BatchResponse {
    pub results: Vec<OperationResult>,
    pub stats: BatchStats,
}

#[derive(Serialize)]
pub struct OperationResult {
    pub index: usize,
    pub operation_type: String,
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Serialize)]
pub struct BatchStats {
    pub total_operations: usize,
    pub successful: usize,
    pub failed: usize,
    pub total_duration_ms: u64,
}
```

---

## 💻 Implementation

```rust
pub async fn handle_batch_operations(
    args: &Map<String, Value>,
    context: &ServerContext,
) -> Result<Value> {
    let req: BatchRequest = serde_json::from_value(
        serde_json::to_value(args)?
    )?;
    
    let start = Instant::now();
    let mut results = Vec::new();
    
    if req.parallel {
        // Execute in parallel
        let tasks: Vec<_> = req.operations.into_iter()
            .enumerate()
            .map(|(index, op)| {
                let ctx = context.clone();
                tokio::spawn(async move {
                    execute_single_operation(index, op, ctx).await
                })
            })
            .collect();
        
        for task in tasks {
            results.push(task.await??);
        }
    } else {
        // Execute sequentially
        for (index, op) in req.operations.into_iter().enumerate() {
            let result = execute_single_operation(
                index, 
                op, 
                context.clone()
            ).await?;
            
            results.push(result);
            
            if !result.success && !req.continue_on_error {
                break;
            }
        }
    }
    
    let stats = BatchStats {
        total_operations: results.len(),
        successful: results.iter().filter(|r| r.success).count(),
        failed: results.iter().filter(|r| !r.success).count(),
        total_duration_ms: start.elapsed().as_millis() as u64,
    };
    
    Ok(serde_json::to_value(BatchResponse {
        results,
        stats,
    })?)
}

async fn execute_single_operation(
    index: usize,
    operation: Operation,
    context: ServerContext,
) -> Result<OperationResult> {
    let start = Instant::now();
    
    let (success, data, error) = match operation.op_type.as_str() {
        "read_file" => {
            match context.sqlite.read_file(&operation.params["file"]).await {
                Ok(content) => (true, Some(json!(content)), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        "get_symbols" => {
            match context.sqlite.get_symbols(&operation.params["file"]).await {
                Ok(symbols) => (true, Some(json!(symbols)), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        "search" => {
            match context.lance.search(&operation.params["query"]).await {
                Ok(results) => (true, Some(json!(results)), None),
                Err(e) => (false, None, Some(e.to_string())),
            }
        }
        _ => (false, None, Some("Unknown operation".into())),
    };
    
    Ok(OperationResult {
        index,
        operation_type: operation.op_type,
        success,
        data,
        error,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}
```

---

## 📈 Success Metrics

- ⚡ 3-5× latency reduction
- ✅ Partial success works
- ⏱️ Parallel execution efficiency > 80%

---

## ✅ Acceptance Criteria

- [ ] Batch multiple operations
- [ ] Parallel execution works
- [ ] continue_on_error works
- [ ] 3× latency reduction
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16
