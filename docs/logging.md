# Логирование

Что пишет gofer, куда, в каком формате, как фильтровать.

Реализация — `src/logger.rs::init` (короткий файл, ~50 строк). Используется `tracing` + `tracing-subscriber` + `tracing-appender`.

## Куда пишутся логи

`~/.gofer/logs/gofer.log.<YYYY-MM-DD>` — rolling daily файл. Автоматически создаётся `~/.gofer/logs/`. Старые файлы не удаляются автоматически — чисти руками либо настрой logrotate.

Помимо файла, для **демона** дополнительно пишется в `~/.gofer/daemon.log` (stdout) и `~/.gofer/daemon.err` (stderr) через перенаправление из `daemonize`. Это не дубль — туда падают любые prints вне `tracing` (например, паники).

## Кто пишет

`logger::init(component)` зовётся для трёх разных компонентов:

| Компонент | Когда инициализируется | Файл логов |
|---|---|---|
| `daemon` | Внутри `run_daemon()` после fork'а | те же rolling-файлы + `daemon.log`/`daemon.err` |
| `mcp` | Внутри `gofer mcp` процесса | rolling-файлы |
| `cli` | Любая CLI-команда кроме `daemon`/`mcp` | rolling-файлы |

В каждой записи есть поле `component = "daemon" | "mcp" | "cli"` — удобно фильтровать.

## Формат

По умолчанию — **JSON** (`tracing-subscriber::fmt::layer().json()`):

```json
{
  "timestamp": "2026-05-28T14:32:11.123456Z",
  "level": "INFO",
  "fields": {
    "message": "Pipeline complete: 152 files processed",
    "files": 152
  },
  "target": "gofer::indexer::pipeline",
  "filename": "src/indexer/pipeline.rs",
  "line_number": 232
}
```

Это можно стримить в Loki/Vector/Filebeat без парсинга текста.

Если хочется человекочитаемого текста — выставь переменную:

```bash
GOFER_LOG_TEXT=1 gofer up
```

Тогда формат будет привычный:

```
2026-05-28T14:32:11.123Z  INFO gofer::indexer::pipeline: Pipeline complete: 152 files processed files=152
```

ANSI-цветов в файле нет (специально — чтобы не засорять).

## Фильтрация

Дефолтный фильтр — `gofer=info` (всё из crate `gofer` на уровне INFO+, остальное молчит).

Переменная `RUST_LOG` переопределяет:

```bash
RUST_LOG=gofer=debug gofer up                              # debug по всему gofer
RUST_LOG=gofer::indexer=trace,gofer=info gofer up          # trace по pipeline, остальное info
RUST_LOG=gofer=info,sqlx=warn gofer up                     # тише sqlx
RUST_LOG=warn gofer up                                     # ТОЛЬКО warn+ (заглушит логи запуска)
```

Синтаксис стандартный `EnvFilter` — поддерживаются модульные пути, уровни, `[span]{field=value}` фильтры.

CLI-флаг `--log-level` — это более простой override; работает как `RUST_LOG=gofer=<level>`:

```bash
gofer --log-level debug up
gofer --log-level warn mcp
```

## Что искать в логах

### Старт

```
INFO gofer::logger: Logging initialized for daemon (PID 12345)
INFO gofer: Tokio runtime configured: 4 worker threads, 8 max blocking threads
INFO gofer: gofer daemon starting (pid 12345)
```

### Активация проекта

```
INFO gofer::daemon::state: Activating project /path/to/project
INFO gofer::indexer::service: Starting full sync for /path/to/project
INFO gofer::indexer::pipeline: Pipeline: 6 parser workers (optimized for CPU usage)
INFO gofer::indexer::embedder: Инициализация embedder pool с внешним API: 4 инстансов
```

### Сам pipeline

```
INFO gofer::indexer::pipeline: Scanner: 152 files sent to pipeline
INFO gofer::indexer::pipeline: Pipeline complete: 152 files processed
```

### Watcher

```
INFO gofer::indexer::watcher: Watching directory: /path/to/project/src (and 12 more)
DEBUG gofer::indexer::watcher: File event: Modify(Data(Content)) for src/main.rs
INFO gofer::indexer::service: Reindexing single file: src/main.rs
```

### Эмбеддер

```
WARN gofer::indexer::embedder: Failed to embed batch (retry 1/3): connection refused
ERROR gofer::indexer::embedder: External embedder error (503): upstream timeout
INFO gofer::error_recovery: Circuit breaker transitioning to half-open
INFO gofer::error_recovery: Circuit breaker recovered (closed)
```

### RPC

```
ERROR gofer::ipc::protocol: RPC Error [-32602]: Invalid params: Symbol name is required
INFO gofer::ipc::server: New connection accepted (active connections: 3)
```

### Shutdown

```
INFO gofer: SIGTERM received, initiating graceful shutdown
INFO gofer::daemon::state: Deactivating all projects
INFO gofer: gofer daemon stopped cleanly
```

## Хвостовая просмотр

```bash
gofer logs -n 100         # последние 100 строк daemon.log
gofer logs -n 200 -f      # follow, как tail -f
gofer logs --err          # daemon.err вместо daemon.log
```

Это смотрит **не** в rolling-файл (`gofer.log.<date>`), а в `daemon.log`/`daemon.err` (perror'ы и панику). Для расшифровки структурированного:

```bash
tail -f ~/.gofer/logs/gofer.log.$(date +%Y-%m-%d) | jq -c 'select(.level=="WARN" or .level=="ERROR")'
```

Если включён `GOFER_LOG_TEXT=1`:

```bash
tail -f ~/.gofer/logs/gofer.log.$(date +%Y-%m-%d) | grep -E 'WARN|ERROR'
```

## Уровни на практике

| Уровень | Когда использовать клиенту | Что увидишь |
|---|---|---|
| `error` | Прод. Только катастрофы. | Сбои эмбеддера, паники, fatal в pipeline. |
| `warn` | Прод/staging. | + Circuit breaker open, retry, скип файла >2 МБ. |
| `info` (дефолт) | Обычная разработка. | + Старт/стоп, активация проекта, сводка sync'а. |
| `debug` | Отладка функционала. | + Каждое событие watcher'а, каждый MCP-вызов, шаги pipeline. |
| `trace` | Глубокая отладка. | + Внутренние шаги парсера, скоринга, sql-запросы (через sqlx). |

На уровне `trace` гофер льёт **много** — терабайт в час на активном проекте легко. Использовать точечно с фильтрами.

## Интеграция с системами агрегации

Поскольку формат — JSON, можно подсасывать любым шипером:

### Vector

```toml
[sources.gofer]
type = "file"
include = ["/home/<user>/.gofer/logs/gofer.log.*"]
read_from = "end"

[transforms.parse_json]
type = "remap"
inputs = ["gofer"]
source = '. = parse_json!(string!(.message))'

[sinks.loki]
type = "loki"
inputs = ["parse_json"]
endpoint = "http://loki:3100"
labels = { component = "{{ .fields.component }}", level = "{{ .level }}" }
```

### Promtail / Loki

```yaml
scrape_configs:
  - job_name: gofer
    static_configs:
      - targets: [localhost]
        labels:
          job: gofer
          __path__: /home/*/.gofer/logs/gofer.log.*
    pipeline_stages:
      - json:
          expressions:
            level: level
            component: fields.component
            message: fields.message
      - labels:
          level:
          component:
```

### jq для быстрых выборок

Все ошибки за день:

```bash
jq -c 'select(.level == "ERROR")' ~/.gofer/logs/gofer.log.$(date +%Y-%m-%d)
```

Сколько проиндексированных файлов в каждом sync'е:

```bash
jq -c 'select(.fields.message | test("Pipeline complete"))' ~/.gofer/logs/gofer.log.*
```

Топ MCP-инструментов по числу ошибок:

```bash
jq -c 'select(.level == "ERROR") | .fields.message' ~/.gofer/logs/gofer.log.* \
  | sort | uniq -c | sort -rn | head -20
```

## Что **не** логируется

- Содержимое файлов целиком. Только пути и фрагменты в сообщениях об ошибках.
- Содержимое аргументов MCP-вызовов в общем логе (они идут в `audit_log` SQLite, не в текстовый лог).
- Эмбеддинги, векторы, сырые SQL-параметры (если только не `RUST_LOG=trace`).
- Секреты, пароли, токены. Если попадутся в `external_api_key` или подобном — это попадёт только в код, не в логи.

## Где смотреть в коде

- `src/logger.rs` — точка инициализации.
- Каждый модуль использует `tracing::{info, warn, error, debug, trace}!` напрямую. Структурированные поля — через `?field` (Debug) и `%field` (Display) синтаксис.
- `tracing-appender` rolling — настройка в `init`, ежедневная ротация.
