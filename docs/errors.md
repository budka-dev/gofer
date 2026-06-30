# Ошибки и коды JSON-RPC

Как gofer репортит сбои клиенту и что делать с каждой категорией. Реализация — `src/error.rs::GoferError`.

## Иерархия

```rust
enum GoferError {
    ParseError(String),
    InvalidParams(String),
    MethodNotFound(String),
    Storage(storage::sqlite::StorageError),
    Lance(storage::lance::LanceError),
    Embedder(indexer::embedder::EmbedderError),
    Parser(indexer::parser::ParserError),
    ToolError(String),
    Internal(anyhow::Error),
}
```

Реальный код:
- `ParseError(String)` — формируется руками там, где парсится что-то приклаженное (например, аргумент-строка инструмента).
- `InvalidParams(String)` — отсутствует обязательный аргумент или его тип не сходится.
- `MethodNotFound(String)` — `tools::dispatch` не нашёл инструмент с таким именем.
- `Storage` / `Lance` / `Embedder` / `Parser` — `#[from]` обёртки над специфическими ошибками подсистем.
- `ToolError(String)` — обобщённая ошибка handler'а, когда специфический вариант не подходит.
- `Internal(anyhow::Error)` — catch-all для всего, что прилетело через `anyhow`. Сюда падает большинство сложных сбоев.

## Маппинг на JSON-RPC коды

`src/error.rs::GoferError::rpc_code()`:

| Вариант | JSON-RPC код | Имя кода |
|---|---|---|
| `ParseError` | **-32700** | Parse error (стандарт) |
| `InvalidParams` | **-32602** | Invalid params (стандарт) |
| `MethodNotFound` | **-32601** | Method not found (стандарт) |
| `Storage`, `Lance`, `Embedder`, `Parser`, `ToolError` | **-32000** | Server error (app-defined) |
| `Internal` | **-32603** | Internal error (стандарт) |

Сообщение в `error.message` — текст `Display`-форматирования варианта (`Embedder error: External embedder error (503): upstream timeout`).

Формат ответа:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "error": {
    "code": -32000,
    "message": "Embedder error: External embedder error (503): ..."
  }
}
```

`error.data` сейчас не заполняется — поле есть в `RpcError`, но всегда `None`.

## Что клиенту делать с каждым кодом

### `-32700` Parse error

Что-то с самим JSON-RPC фреймом или с дополнительным парсингом аргумента-строки. Случается редко — обычно клиент сам генерирует валидный JSON. Если повторяется, скорее всего, баг в сериализации на стороне клиента.

### `-32601` Method not found

Имя инструмента не маршрутизируется. Возможные причины:
- Опечатка (`get_symbol` вместо `get_symbols`).
- Инструмент реализован в handler, но не зарегистрирован в `dispatch` — проверь `src/daemon/tools.rs::dispatch`.
- Старый клиент кеширует прошлый `tools/list`. Перезапросить.

**Действие:** клиент должен либо исправить имя, либо запросить `tools/list` повторно.

### `-32602` Invalid params

Сообщение содержит детали (`Symbol name is required`, `file is required`). Большая часть таких ошибок исправима со стороны клиента — добавь недостающий параметр.

**Действие:** показать пользователю, какой параметр пропущен, либо использовать дефолт.

### `-32000` Server error (`Storage`/`Lance`/`Embedder`/`Parser`/`ToolError`)

Кодом разбавляется по подсистемам:

- **Embedder error** → проверить, поднят ли HTTP-эмбеддер; смотреть `troubleshooting.md::Эмбеддер недоступен`. После 5 подряд ошибок размыкается `embedding_circuit` на 30 с — клиенту лучше отбэкоффиться и повторить позже.
- **Storage error** → SQLite-проблема: блокировка БД, нарушение FK, нехватка диска. Подробности в `daemon.log`.
- **Lance error** → векторное хранилище: размер вектора не совпадает с `dimensions`, корраптнутый фрагмент. Часто решается `gofer reindex --force`.
- **Parser error** → tree-sitter не смог распарсить файл. Чаще всего из-за wasm ABI mismatch (см. `gofer install-lang`) или невалидного синтаксиса в исходнике.
- **Tool error** → специфическое для инструмента. Сообщение даёт деталь.

**Действие:** обычно retry с экспоненциальным бэкоффом. Если повторяется — проверить логи демона.

### `-32603` Internal error

Catch-all. Сюда падают: незахваченные `anyhow::Error` из глубины, паники, неожиданные I/O сбои.

**Действие:** клиенту повторить запрос один раз; если повторяется — собрать `daemon.log` за момент сбоя и завести issue.

## Циркуляр-брейкер ошибки

Когда `embedding_circuit` или `vector_circuit` в состоянии `Open`, любой вызов через них возвращает ошибку с сообщением `Circuit breaker open`. Класс ошибки — тот же, что у подсистемы (`Embedder`/`Internal`), код RPC — **-32000**.

Поведение клиента:

- Подожди cooldown (30 с для embedding, 10 с для vector — см. `architecture.md#circuit-breakers`).
- Не уменьшай интервал — счётчик закрытия начнёт ткать только после успешных проб в `HalfOpen`.

## Подсистемные ошибки (детализация)

### `storage::sqlite::StorageError`

Перечисление с вариантами от `sqlx::Error`, миграции, отсутствие записи. Все мапятся в `-32000`.

### `storage::lance::LanceError`

Аналогично — обёртка над `lancedb::Error`, плюс собственные ошибки несоответствия схеме (например, `embedding dimension mismatch`).

### `indexer::embedder::EmbedderError`

Сейчас только один вариант: `Embedding(anyhow::Error)`. Сообщение содержит HTTP-статус и тело ответа, если эмбеддер вернул не-2xx.

### `indexer::parser::ParserError`

Сбои tree-sitter: грамматика не найдена, wasm-инициализация, query-несоответствие. Чаще всего лечится `gofer install-lang <name>`.

## Логирование ошибок

Все ошибки RPC автоматически логируются на уровне `error` через `tracing` при формировании `DaemonResponse::error` (`src/ipc/protocol.rs:54`). В логе будет:

```
2026-05-28T14:32:11.123Z ERROR gofer::ipc::protocol: RPC Error [-32000]: Embedder error: External embedder error (503): upstream timeout
```

Так что для большинства проблем диагностика клиент-сайд через `error.message` + сервер-сайд через `gofer logs --err` достаточна.

## Что ошибки **не** делают

- Не падают на panic — все handler'ы возвращают `Result<Value>`, паника в задаче не уронит демон, но запрос ответит `-32603`.
- Не блокируют демон — каждый RPC обрабатывается в отдельной задаче.
- Не закрывают сокет — клиент может слать следующий запрос на той же сессии.
- Не пишут в трассу `error.data` — это поле существует, но gofer его не заполняет (если очень нужно, патчь `DaemonResponse::error`).
