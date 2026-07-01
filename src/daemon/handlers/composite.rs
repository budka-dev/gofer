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

fn is_type_like(kind: SymbolKind) -> bool {
    // trait/struct/enum/interface/class/type — у которых бывают impl'ы
    !matches!(kind, SymbolKind::Function)
}

/// Взять массив по ключу из результата dispatch, капнув до cap. Пусто при ошибке.
///
/// Вызов `tools::dispatch` заворачивается в `Box::pin`, чтобы разорвать
/// цикл рекурсии типов future (`dispatch` -> `tool_explain_symbol` -> `dispatch`),
/// иначе компилятор не может вычислить размер состояния async fn.
async fn dispatch_array(ctx: &ToolContext, tool: &str, args: Value, key: &str, cap: usize) -> Value {
    match Box::pin(tools::dispatch(tool, args, ctx)).await {
        Ok(v) => {
            let arr = v
                .get(key)
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
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
    let include_bodies = args
        .get("include_bodies")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

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
            .map(|s| {
                json!({ "file": make_relative(&ctx.root_path, &s.file_path), "line": s.line, "kind": s.kind })
            })
            .collect();
        return Ok(json!({ "symbol": symbol, "candidates": candidates }));
    }
    let def = &hits[0];

    // 2. Callers — из storage (тот же источник, что get_callers).
    let refs = ctx.sqlite.get_references_by_name(symbol).await?;
    let callers = project_callers(&refs, &ctx.root_path, MAX_LIST);

    // 3. Callees — переиспользуем логику get_callees.
    let callees = dispatch_array(
        ctx,
        "get_callees",
        json!({ "symbol": symbol, "file": def.file_path }),
        "callees",
        MAX_LIST,
    )
    .await;

    // 4. Implementations — только для type-like символов.
    let implementations = if is_type_like(def.kind) {
        dispatch_array(
            ctx,
            "find_implementations",
            json!({ "name": symbol }),
            "implementations",
            MAX_LIST,
        )
        .await
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

    // 5. Тело — только по флагу. read_function_context ожидает {file, function},
    // где file — путь относительно root_path (он резолвится через ctx.root_path.join(file)).
    if include_bodies {
        if let Ok(body) = Box::pin(tools::dispatch(
            "read_function_context",
            json!({
                "file": make_relative(&ctx.root_path, &def.file_path),
                "function": symbol,
            }),
            ctx,
        ))
        .await
        {
            out["body"] = body;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::project_callers;
    use crate::models::chunk::ReferenceWithPath;
    use std::path::Path;

    fn r(kind: &str, path: &str, line: i32) -> ReferenceWithPath {
        ReferenceWithPath {
            id: 0,
            target_name: "dispatch".into(),
            ref_kind: kind.into(),
            line,
            file_path: path.into(),
        }
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
        assert_eq!(v["count"], 30); // полный счётчик
        assert_eq!(v["top"].as_array().unwrap().len(), 20); // капнуто
    }
}
