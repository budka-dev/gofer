# Composite answer-tools: explain_symbol + where_used

**Дата:** 2026-07-01
**Статус:** утверждён, ожидает плана реализации
**Ветка:** main

## Цель

Добавить в gofer (read-only index-search MCP) два **first-class composite-инструмента**,
которые собирают ответ на частое намерение агента за **один round-trip**, вместо
3–4 отдельных вызовов, которые агент склеивает сам:

- `explain_symbol` — «пойми этот символ»;
- `where_used` — «где и как он используется».

Это прямой рост токен-эффективности (меньше раундов, курированная компактная
проекция) — главная ниша gofer.

### Мотивация

- В `src/ipc/server.rs` уже живут композиции (`prompt_explain_module` =
  skeleton+symbols+deps, `prompt_find_related` = search+search_by_purpose), но
  как MCP-**prompts**, не как **tools** — агент не может вызвать их через
  `tools/call` и получить ответ за один заход.
- Гранулярные символьные тулы (`get_symbols`/`get_references`/`get_callers`/
  `get_callees`/`find_implementations`) отдают сырые выдачи; агент тратит
  несколько раундов и токенов на их сборку.

## Ключевой принцип: компактная проекция, не конкатенация

Наивный композит, склеивающий сырые выхлопы под-тулов (полные тела + все ссылки),
будет **больше** суммы отдельных вызовов и сломает саму идею экономии. Поэтому:

- списки **капятся** (top-N + `has_more`/`count`);
- тела кода отдаются **только по флагу** `include_bodies`;
- каждая секция — проекция нужных полей (имя + `file:line` + сигнатура), а не
  passthrough полного JSON под-тула.

## Подход (вариант A)

Новый хендлер `src/daemon/handlers/composite.rs` с двумя функциями. Каждая
**композирует существующие тулы через `tools::dispatch(name, args, ctx)`** —
переиспользует отлаженную логику, не дублирует её и не трогает контракты
существующих тулов. (Паттерн уже используется в `server.rs`:
`tools::dispatch("skeleton", json!({...}), ctx).await?`.)

Отвергнутые альтернативы:
- **B:** флаг `explain=true` на `get_symbols` — ломает контракт существующего тула.
- **C:** оставить как MCP-prompts — статус-кво, нет `tools/call` за один round-trip.

## Контракты инструментов (v1)

Оба read-only. Дизамбигуация общая: вход `symbol` (required) + опц. `file`.
Если символ резолвится в несколько определений и `file` не задан — возвращаем
`candidates[]` (list of `{file,line,kind}`), **НЕ угадываем**. Если символ не
найден — ошибка `InvalidParams` с ближайшими совпадениями (как в `symbol_exists`).

### `explain_symbol`

**Вход:** `symbol` (string, req), `file` (string, opt — дизамбигуация),
`include_bodies` (bool, default false).

**Композирует:** `get_symbols`/`symbol_exists` (локация + kind + signature) →
`read_function_context` (тело/док, только при `include_bodies`) →
`get_callees` (что вызывает) → `get_callers` (счётчик + top-N) →
`find_implementations` (только если trait/interface).

**Выход (компактный JSON):**
```json
{
  "symbol": "dispatch",
  "definition": {
    "file": "src/daemon/tools.rs", "line": 12, "kind": "function",
    "signature": "pub async fn dispatch(name: &str, args: Value, ctx: &ToolContext) -> Result<Value>"
  },
  "doc": "Dispatch a tool call by name...",
  "callers": { "count": 8, "top": ["src/ipc/server.rs:639", "..."] },
  "callees": ["search::tool_search", "symbols::tool_get_symbols", "..."],
  "implementations": [],
  "body": "..."
}
```
- `doc` — ведущий doc-комментарий определения, если доступен дёшево; иначе опустить.
- `implementations` присутствует только если символ — trait/interface/тип с impl'ами.
- `body` присутствует только при `include_bodies=true`.
- `callers.top` и `callees` капятся (константа `MAX_LIST`, напр. 20) с полем
  `callers.count` = полное число.

### `where_used`

**Вход:** `symbol` (string, req), `file` (string, opt), `depth` (int, default 1 —
прямые использования; >1 — транзитивные пути к точкам входа).

**Композирует:** `get_references` (сгруппировать по файлу) → `get_callers`
(прямые) → при `depth>1` `dependency_subgraph` (обратная достижимость к
entrypoints, ограничена глубиной).

**Выход (компактный JSON):**
```json
{
  "symbol": "dispatch",
  "references": { "count": 42, "by_file": { "src/ipc/server.rs": [639, 750, 752] } },
  "callers": { "count": 8, "direct": ["src/ipc/server.rs:639", "..."] },
  "reaches_entrypoints": [["main", "activate_project", "dispatch"]]
}
```
- `references.by_file` капнуто по числу файлов и по числу строк на файл; `count` —
  полное число.
- `reaches_entrypoints` присутствует только при `depth>1`; каждый элемент — путь
  имён от точки входа к символу, число путей ограничено.

## Точки правки

| Файл | Изменение |
|---|---|
| `src/daemon/handlers/composite.rs` | **Создать**: `tool_explain_symbol`, `tool_where_used` + приватные проекторы/константы капов |
| `src/daemon/handlers/mod.rs` | `+ pub mod composite;` |
| `src/daemon/tools.rs` | `dispatch()`: +2 ветки; `core_tools_list()`: +2 схемы |

Итог: **35 → 37 инструментов**. Существующие тулы и их контракты не трогаются.

## Обработка ошибок

- `symbol` отсутствует/пуст → `InvalidParams("symbol is required")`.
- Символ не найден → `InvalidParams` с полем ближайших совпадений (переиспользовать
  логику fuzzy из `symbol_exists`, если доступна; иначе — просто сообщение).
- Несколько определений без `file` → успешный ответ с `candidates[]` (не ошибка),
  без остальных секций.
- Под-вызов `dispatch` вернул ошибку → секция опускается или помечается `null`;
  композит не падает целиком из-за одной пустой секции (кроме отсутствия
  определения — тогда осмысленного ответа нет).

## Тестирование и верификация

- **Юнит-тесты** в духе существующих символьных хендлеров (`#[cfg(test)]` в
  `symbols.rs`) — проверить проекцию/капы/дизамбигуацию на фикстур-индексе, если
  существующие тесты дают такой каркас; иначе — минимальные тесты чистых
  функций-проекторов (без БД).
- **Живой smoke** через MCP-сокет (как при верификации рефакторинга):
  - `explain_symbol {symbol:"dispatch"}` на самом gofer → `definition` указывает
    на `src/daemon/tools.rs:12`, `callers.count`≈8, `callees` непустой;
  - `where_used {symbol:"dispatch"}` → `references`/`callers` непустые, счётчики
    совпадают с прямыми `get_references`/`get_callers`;
  - дизамбигуация: символ с несколькими определениями без `file` → `candidates[]`;
  - `tools/list` → 37 инструментов.
- `cargo build` 0 warnings, `cargo test` зелёный.

## Вне объёма (YAGNI, возможные follow-up)

- `explain_file` (файловый обзор) — отдельная итерация.
- Token-accounting в ответе (`tokens_saved`) — отдельный вектор роста.
- `format=markdown` режим — пока только компактный JSON.
- Транзитивные callee-деревья глубиной >1 в `explain_symbol` — v1 только прямые.
