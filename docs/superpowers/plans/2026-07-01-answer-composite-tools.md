# Composite answer-tools (explain_symbol, where_used) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Добавить два read-only composite-инструмента (`explain_symbol`, `where_used`), собирающих компактный ответ за один round-trip из данных индекса.

**Architecture:** Новый хендлер `handlers/composite.rs`. Структурированные секции (definition, callers, references) собираются **напрямую из storage** (`ctx.sqlite.*`) — это даёт компактную проекцию без хрупкого парсинга человекочитаемых строк под-тулов; нетривиальные секции (callees, implementations, body) переиспользуются через `tools::dispatch`, беря их уже готовый массив/значение. Чистые проектор-функции юнит-тестируются; композиция верифицируется живым MCP-smoke.

**Tech Stack:** Rust 2021, tokio, sqlx (SQLite), serde_json. Сборка `cargo build`, тесты `cargo test`.

**Спека:** `docs/superpowers/specs/2026-07-01-answer-composite-tools-design.md`

## Global Constraints

- Read-only: инструменты не мутируют файлы и не спавнят процессы.
- Компактная проекция: списки капятся до `MAX_LIST = 20` с полным `count`; тела кода — только по `include_bodies`.
- Дизамбигуация: `symbol` (req) + опц. `file`; >1 определения без `file` → `candidates[]` (не ошибка, не угадывать); не найдено → `InvalidParams`.
- Существующие тулы и их контракты не трогать. Итог: 35 → 37 инструментов.
- После каждой задачи `cargo build` 0 warnings и `cargo test` зелёный.
- Стиль/имена — как в `handlers/symbols.rs` (напр. `make_relative(&ctx.root_path, &path)`).

---

## Верифицированные факты о кодовой базе (ground truth)

- Storage-методы (в `src/storage/sqlite.rs`):
  - `search_symbols_with_path(&self, query: &str, limit: i32) -> Result<Vec<SymbolWithPath>>` (может быть неточным/fuzzy → фильтровать по `name == symbol`).
  - `get_references_by_name(&self, name: &str) -> Result<Vec<ReferenceWithPath>>`.
- Типы (в `src/models/chunk.rs`):
  - `SymbolWithPath { id:i64, name:String, kind:SymbolKind, line:i32, end_line:i32, signature:Option<String>, file_path:String }`.
  - `ReferenceWithPath { id:i64, target_name:String, ref_kind:String, line:i32, file_path:String }`.
  - `SymbolKind` — enum, сериализуется в snake_case ("function","struct",...).
- Паттерн композиции: `tools::dispatch(name, json!({...}), ctx).await` (см. `src/ipc/server.rs`).
- `get_callees` возвращает JSON с полем `"callees": [...]`; `find_implementations` — с полем `"implementations": [...]` (ПОДТВЕРДИТЬ точные ключи по выводу тула перед проецированием; если ключ иной — взять фактический массив).
- `make_relative(root: &Path, path: &str) -> String` — в `handlers/common.rs` (используется в `symbols.rs`).
- Регистрация тула: ветка в `dispatch()` + `json!` схема в `core_tools_list()` в `src/daemon/tools.rs`; объявление модуля в `src/daemon/handlers/mod.rs`.
- Тесты в проекте — чистые юнит-тесты (пример: `mod tests` в `symbols.rs:1360`, тестирует `split_signature`). БД-фикстур нет → юнитим только чистые проекторы.

---

## Task 1: `explain_symbol` (+ scaffold composite.rs + чистые проекторы)

**Files:**
- Create: `src/daemon/handlers/composite.rs`
- Modify: `src/daemon/handlers/mod.rs` (+`pub mod composite;`)
- Modify: `src/daemon/tools.rs` (dispatch-ветка + схема)
- Test: юнит-тесты внутри `composite.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub async fn tool_explain_symbol(args: Value, ctx: &ToolContext) -> Result<Value>`; чистый хелпер `pub(crate) fn project_callers(refs: &[ReferenceWithPath], root: &std::path::Path, cap: usize) -> serde_json::Value`; константа `MAX_LIST`.

- [ ] **Step 1: Написать падающий тест на чистый проектор `project_callers`**

Создать `src/daemon/handlers/composite.rs` с тест-модулем:
```rust
#[cfg(test)]
mod tests {
    use super::project_callers;
    use crate::models::chunk::ReferenceWithPath;
    use std::path::Path;

    fn r(kind: &str, path: &str, line: i32) -> ReferenceWithPath {
        ReferenceWithPath { id: 0, target_name: "dispatch".into(), ref_kind: kind.into(), line, file_path: path.into() }
    }

    #[test]
    fn caps_and_counts_only_calls_and_usages() {
        let refs = vec![
            r("call", "/repo/src/a.rs", 10),
            r("usage", "/repo/src/b.rs", 20),
            r("import", "/repo/src/c.rs", 30), // must be excluded
        ];
        let v = project_callers(&refs, Path::new("/repo"), 20);
        assert_eq!(v["count"], 2);
        let top = v["top"].as_array().unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0], "src/a.rs:10");
    }

    #[test]
    fn respects_cap() {
        let refs: Vec<_> = (0..30).map(|i| r("call", "/repo/src/a.rs", i)).collect();
        let v = project_callers(&refs, Path::new("/repo"), 20);
        assert_eq!(v["count"], 30);          // полный счётчик
        assert_eq!(v["top"].as_array().unwrap().len(), 20); // капнуто
    }
}
```

- [ ] **Step 2: Запустить тест — убедиться, что не компилируется/падает**

Run: `cargo test -p gofer project_callers 2>&1 | tail -15`
Expected: FAIL — `cannot find function project_callers`.

- [ ] **Step 3: Реализовать `project_callers` + шапку модуля**

В начало `src/daemon/handlers/composite.rs` (перед тест-модулем):
```rust
//! Composite answer-tools: explain_symbol, where_used.
//! Собирают компактный ответ за один round-trip из данных индекса.

use super::common::{make_relative, ToolContext};
use crate::daemon::tools;
use crate::error::GoferError;
use crate::models::chunk::{ReferenceWithPath, SymbolKind, SymbolWithPath};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

const MAX_LIST: usize = 20;

/// Проекция ссылок-вызовов в компактное {count, top:["file:line"]}.
pub(crate) fn project_callers(refs: &[ReferenceWithPath], root: &Path, cap: usize) -> Value {
    let items: Vec<String> = refs
        .iter()
        .filter(|r| r.ref_kind == "call" || r.ref_kind == "usage")
        .map(|r| format!("{}:{}", make_relative(root, &r.file_path), r.line))
        .collect();
    let count = items.len();
    let top: Vec<String> = items.into_iter().take(cap).collect();
    json!({ "count": count, "top": top })
}
```
Если `make_relative` принимает `&Arc<PathBuf>`/иную сигнатуру — привести тип (в `symbols.rs` вызывается как `make_relative(&ctx.root_path, &r.file_path)`; для чистого хелпера принять `&Path` и в вызывающем коде передать `ctx.root_path.as_path()` или `&ctx.root_path`).

- [ ] **Step 4: Тест зелёный**

Run: `cargo test -p gofer project_callers 2>&1 | tail -15`
Expected: PASS (2 теста).

- [ ] **Step 5: Реализовать `tool_explain_symbol`**

Добавить в `composite.rs` (после хелпера). ПЕРЕД написанием — подтвердить точные ключи вывода `get_callees`/`find_implementations` и аргументы `read_function_context`:
`grep -nE '"callees"|"implementations"' src/daemon/handlers/symbols.rs` и `grep -nA6 'fn tool_read_function_context' src/daemon/handlers/files.rs`.
```rust
fn is_type_like(kind: SymbolKind) -> bool {
    // trait/struct/enum/interface/class/type — у которых бывают impl'ы
    !matches!(kind, SymbolKind::Function)
}

/// Взять массив по ключу из результата dispatch, капнув до cap. Пусто при ошибке.
async fn dispatch_array(ctx: &ToolContext, tool: &str, args: Value, key: &str, cap: usize) -> Value {
    match tools::dispatch(tool, args, ctx).await {
        Ok(v) => {
            let arr = v.get(key).and_then(|x| x.as_array()).cloned().unwrap_or_default();
            Value::Array(arr.into_iter().take(cap).collect())
        }
        Err(_) => json!([]),
    }
}

pub async fn tool_explain_symbol(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("symbol is required".into()).into());
    }
    let file = args.get("file").and_then(|v| v.as_str());
    let include_bodies = args.get("include_bodies").and_then(|v| v.as_bool()).unwrap_or(false);

    // 1. Локация определения(ий): точное совпадение имени.
    let mut hits: Vec<SymbolWithPath> = ctx
        .sqlite
        .search_symbols_with_path(symbol, 50)
        .await?
        .into_iter()
        .filter(|s| s.name == symbol)
        .collect();
    if let Some(f) = file {
        hits.retain(|s| {
            let rel = make_relative(&ctx.root_path, &s.file_path);
            rel == f || s.file_path.ends_with(f)
        });
    }
    if hits.is_empty() {
        return Err(GoferError::InvalidParams(format!("symbol '{}' not found", symbol)).into());
    }
    if hits.len() > 1 && file.is_none() {
        let candidates: Vec<Value> = hits
            .iter()
            .map(|s| json!({ "file": make_relative(&ctx.root_path, &s.file_path), "line": s.line, "kind": s.kind }))
            .collect();
        return Ok(json!({ "symbol": symbol, "candidates": candidates }));
    }
    let def = &hits[0];

    // 2. Callers — из storage (тот же источник, что get_callers).
    let refs = ctx.sqlite.get_references_by_name(symbol).await?;
    let callers = project_callers(&refs, &ctx.root_path, MAX_LIST);

    // 3. Callees — переиспользуем логику get_callees.
    let callees = dispatch_array(
        ctx, "get_callees",
        json!({ "symbol": symbol, "file": def.file_path }),
        "callees", MAX_LIST,
    ).await;

    // 4. Implementations — только для type-like символов.
    let implementations = if is_type_like(def.kind) {
        dispatch_array(ctx, "find_implementations", json!({ "name": symbol }), "implementations", MAX_LIST).await
    } else {
        json!([])
    };

    let mut out = json!({
        "symbol": symbol,
        "definition": {
            "file": make_relative(&ctx.root_path, &def.file_path),
            "line": def.line,
            "kind": def.kind,
            "signature": def.signature,
        },
        "callers": callers,
        "callees": callees,
        "implementations": implementations,
    });

    // 5. Тело — только по флагу.
    if include_bodies {
        if let Ok(body) = tools::dispatch(
            "read_function_context",
            json!({ "symbol": symbol, "file": def.file_path }),
            ctx,
        ).await {
            out["body"] = body;
        }
    }
    Ok(out)
}
```
Примечание: аргументы `read_function_context` привести к его фактической сигнатуре (Step 5 grep). Если он требует `{file, line}` — передать `def.file_path` + `def.line`.

- [ ] **Step 6: Зарегистрировать модуль и тул**

В `src/daemon/handlers/mod.rs` добавить (в алфавитном порядке рядом с `common`): `pub mod composite;`.

В `src/daemon/tools.rs`:
- в `dispatch()` добавить ветку:
  ```rust
  "explain_symbol" => composite::tool_explain_symbol(args, ctx).await,
  ```
- в `core_tools_list()` добавить схему:
  ```rust
  json!({
      "name": "explain_symbol",
      "description": "Explain a symbol in one call: definition, signature, callers (count+top), callees, and implementations (for types). Compact projection; full body only with include_bodies. Cheaper than separate get_symbols/get_callers/get_callees calls.",
      "inputSchema": {
          "type": "object",
          "properties": {
              "symbol": { "type": "string", "description": "Symbol name to explain" },
              "file": { "type": "string", "description": "Optional file to disambiguate when the name is defined in multiple places" },
              "include_bodies": { "type": "boolean", "description": "Include the full definition body (default false)", "default": false }
          },
          "required": ["symbol"]
      }
  }),
  ```

- [ ] **Step 7: Сборка + юнит-тесты**

Run: `cargo build 2>&1 | tail -8 && cargo test -p gofer composite 2>&1 | tail -10`
Expected: build SUCCESS, 0 warnings; тесты `project_callers` PASS.

- [ ] **Step 8: Живой MCP-smoke**

Пересобрать/переустановить бинарь и перезапустить демон (как в верификации):
```bash
cargo install --path . --force && ~/.cargo/bin/gofer down && ~/.cargo/bin/gofer up && sleep 3 && ~/.cargo/bin/gofer start
```
Драйвить сокет (скрипт-драйвер из верификации или python one-liner):
- `tools/list` → 36 инструментов, среди них `explain_symbol`.
- `explain_symbol {symbol:"dispatch"}` → `definition.file` = `src/daemon/tools.rs`, `definition.line` ≈ 12, `callers.count` > 0, `callees` — массив.
Захватить JSON-ответ как evidence.

- [ ] **Step 9: Коммит**

```bash
git add src/daemon/handlers/composite.rs src/daemon/handlers/mod.rs src/daemon/tools.rs
git commit -m "feat(tools): add explain_symbol composite tool"
```

---

## Task 2: `where_used`

**Files:**
- Modify: `src/daemon/handlers/composite.rs` (добавить хелпер + тул)
- Modify: `src/daemon/tools.rs` (ветка + схема)
- Test: юнит-тест в `composite.rs`

**Interfaces:**
- Consumes: `MAX_LIST`, типы из Task 1.
- Produces: `pub async fn tool_where_used(args: Value, ctx: &ToolContext) -> Result<Value>`; чистый хелпер `pub(crate) fn project_references_by_file(refs: &[ReferenceWithPath], root: &std::path::Path, file_cap: usize, line_cap: usize) -> serde_json::Value`.

- [ ] **Step 1: Падающий тест на `project_references_by_file`**

Добавить в тест-модуль `composite.rs`:
```rust
    #[test]
    fn groups_references_by_file_with_counts() {
        use super::project_references_by_file;
        let refs = vec![
            r("call", "/repo/src/a.rs", 10),
            r("usage", "/repo/src/a.rs", 12),
            r("call", "/repo/src/b.rs", 5),
        ];
        let v = project_references_by_file(&refs, std::path::Path::new("/repo"), 20, 20);
        assert_eq!(v["count"], 3);
        let by_file = v["by_file"].as_object().unwrap();
        assert_eq!(by_file["src/a.rs"].as_array().unwrap().len(), 2);
        assert_eq!(by_file["src/b.rs"].as_array().unwrap(), &vec![serde_json::json!(5)]);
    }
```

- [ ] **Step 2: Запустить — убедиться, что падает**

Run: `cargo test -p gofer project_references_by_file 2>&1 | tail -12`
Expected: FAIL — `cannot find function project_references_by_file`.

- [ ] **Step 3: Реализовать `project_references_by_file`**

Добавить в `composite.rs`:
```rust
/// Проекция всех ссылок в {count, by_file:{ "rel/path": [line,...] }} с капами.
pub(crate) fn project_references_by_file(
    refs: &[ReferenceWithPath],
    root: &Path,
    file_cap: usize,
    line_cap: usize,
) -> Value {
    use std::collections::BTreeMap;
    let count = refs.len();
    let mut by_file: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for r in refs {
        let rel = make_relative(root, &r.file_path);
        by_file.entry(rel).or_default().push(r.line as i64);
    }
    let mut map = serde_json::Map::new();
    for (file, mut lines) in by_file.into_iter().take(file_cap) {
        lines.truncate(line_cap);
        map.insert(file, json!(lines));
    }
    json!({ "count": count, "by_file": Value::Object(map) })
}
```

- [ ] **Step 4: Тест зелёный**

Run: `cargo test -p gofer project_references_by_file 2>&1 | tail -12`
Expected: PASS.

- [ ] **Step 5: Реализовать `tool_where_used`**

Добавить в `composite.rs`:
```rust
pub async fn tool_where_used(args: Value, ctx: &ToolContext) -> Result<Value> {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if symbol.is_empty() {
        return Err(GoferError::InvalidParams("symbol is required".into()).into());
    }
    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1);

    let refs = ctx.sqlite.get_references_by_name(symbol).await?;
    if refs.is_empty() {
        return Err(GoferError::InvalidParams(format!("no references found for '{}'", symbol)).into());
    }
    let references = project_references_by_file(&refs, &ctx.root_path, MAX_LIST, MAX_LIST);
    let callers = project_callers(&refs, &ctx.root_path, MAX_LIST);

    let mut out = json!({
        "symbol": symbol,
        "references": references,
        "callers": callers,
    });

    // Транзитивные пути к точкам входа — только при depth>1, переиспользуем dependency_subgraph.
    if depth > 1 {
        if let Ok(sub) = tools::dispatch(
            "dependency_subgraph",
            json!({ "symbol": symbol, "depth": depth, "direction": "callers" }),
            ctx,
        ).await {
            out["reaches_entrypoints"] = sub;
        }
    }
    Ok(out)
}
```
Примечание: аргументы `dependency_subgraph` (`symbol`/`depth`/`direction`) привести к его фактической сигнатуре — перед реализацией `grep -nA8 'fn tool_dependency_subgraph' src/daemon/handlers/symbols.rs`. Если он не поддерживает обратное направление — оставить `reaches_entrypoints` только с тем, что тул реально отдаёт, или опустить секцию (не падать).

- [ ] **Step 6: Зарегистрировать тул**

В `src/daemon/tools.rs`:
- в `dispatch()`:
  ```rust
  "where_used" => composite::tool_where_used(args, ctx).await,
  ```
- в `core_tools_list()`:
  ```rust
  json!({
      "name": "where_used",
      "description": "Find where a symbol is used in one call: references grouped by file (count + lines) and direct callers. With depth>1, adds transitive caller paths toward entrypoints. Compact projection.",
      "inputSchema": {
          "type": "object",
          "properties": {
              "symbol": { "type": "string", "description": "Symbol name" },
              "file": { "type": "string", "description": "Optional file to disambiguate" },
              "depth": { "type": "integer", "description": "1 = direct uses (default); >1 = include transitive caller paths", "default": 1 }
          },
          "required": ["symbol"]
      }
  }),
  ```

- [ ] **Step 7: Сборка + тесты**

Run: `cargo build 2>&1 | tail -8 && cargo test -p gofer composite 2>&1 | tail -10`
Expected: build SUCCESS 0 warnings; тесты PASS.

- [ ] **Step 8: Живой MCP-smoke**

Переустановить бинарь + рестарт демона (как в Task 1 Step 8). Драйвить:
- `tools/list` → 37 инструментов, есть `where_used`.
- `where_used {symbol:"dispatch"}` → `references.count` > 0, `references.by_file` непустой, `callers.count` совпадает с `explain_symbol` для того же символа.
Захватить JSON как evidence.

- [ ] **Step 9: Коммит**

```bash
git add src/daemon/handlers/composite.rs src/daemon/tools.rs
git commit -m "feat(tools): add where_used composite tool"
```

---

## Task 3: Документация + память + финальный smoke

**Files:**
- Modify: `docs/tools-reference.md`
- Modify: `/home/e5ash/.claude/projects/-home-e5ash-storage-vibe-gofer/memory/project_gofer.md`

- [ ] **Step 1: Добавить оба тула в `docs/tools-reference.md`**

Добавить раздел «Composite / answer-tools» с описанием `explain_symbol` и `where_used`: назначение, входные аргументы (`symbol`, `file?`, `include_bodies?`/`depth?`), формат компактного JSON-ответа, и пометка «cheaper than N separate calls». Обновить счётчик инструментов, если он в файле указан (35 → 37).

- [ ] **Step 2: Обновить память проекта**

В `project_gofer.md` дописать: добавлены composite-тулы `explain_symbol`/`where_used` (37 инструментов); собираются в `handlers/composite.rs` из storage + `dispatch`.

- [ ] **Step 3: Финальный smoke (37 инструментов, оба тула)**

Драйвить сокет:
- `tools/list` → **37**.
- `explain_symbol {symbol:"tool_where_used"}` → корректное определение в `composite.rs`.
- `where_used {symbol:"project_callers"}` → references включают `composite.rs`.
Захватить вывод.

- [ ] **Step 4: Коммит**

```bash
git add docs/tools-reference.md
git commit -m "docs: document explain_symbol and where_used composite tools"
```

---

## Итоговая верификация

- `cargo build` 0 warnings; `cargo test` зелёный (включая новые проектор-тесты).
- `tools/list` через MCP-сокет → 37 инструментов, включая `explain_symbol` и `where_used`.
- Оба тула на живом gofer возвращают компактный корректный JSON; `callers.count` консистентен между `explain_symbol` и `where_used` для одного символа.
- Существующие 35 инструментов не затронуты.
