# gofer

MCP-сервер на Rust для индексации и навигации по кодовым базам. Предоставляет AI-ассистентам токен-эффективный доступ к коду через AST-парсинг, семантический поиск и компактные ответы.

- Версия: `0.1.0` (MVP)
- Toolchain: Rust 2021, проверено на `1.93.0`
- Лицензия: см. `LICENSE`

## Что внутри

- Фоновый демон, который держит индекс в памяти и слушает Unix-сокет (`~/.gofer/daemon.sock`).
- Stdio-мост `gofer mcp`, который подключают MCP-клиенты (Claude Code, Qoder и др.).
- Гибридное хранилище: SQLite для метаданных и символов, LanceDB для векторов.
- AST через tree-sitter с динамически загружаемыми wasm-грамматиками (`gofer install-lang <name>`).
- Интеграции с LSP (rust-analyzer, pyright, typescript-language-server и т. д.).
- 70+ MCP-инструментов: поиск, чтение, символы, git, диагностика, sandbox, буфер обмена, batch.

Подробнее: [docs/](docs/).

## Зачем

gofer заточен под одну вещь: дать AI-ассистенту ответ на вопрос про код, **потратив минимум токенов**. Несколько ключевых приёмов:

- `skeleton` — файл без тел функций, экономит 3–5× против полного `read_file`.
- `read_function_context` — одна функция вместе с её типами/импортами, экономит 90–95% относительно чтения файла.
- `read_types_only` — только структуры данных, без логики.
- `context_bundle skeleton_deps_only=true` — главный файл целиком, его зависимости как скелеты.
- `clipboard_copy/paste` (CAS-буфер) — короткий hash вместо повторной отправки кода, защита от галлюцинаций при копировании.
- `batch_operations` — N read/search-вызовов одним RPC, latency падает в 3–5×.

См. [docs/examples.md](docs/examples.md) — реальные сценарии «что вместо чего». Сравнения с native инструментами лежат в `tests/*_comparison.md`.

## Установка

Требуется Rust toolchain (рекомендуется stable ≥ 1.90) и стандартные системные зависимости для `git2`, `lancedb` и `reqwest`.

```bash
cargo install --path . --locked
# либо
cargo build --release
cp target/release/gofer ~/.cargo/bin/
```

Бинарь устанавливается под именем `gofer`.

### Внешний эмбеддер

gofer не считает эмбеддинги локально — он POSTит батчи текстов на HTTP-эндпоинт. По умолчанию это `http://127.0.0.1:8080/embed/`. URL и модель задаются в проектном конфиге (см. ниже).

Совместим любой сервис, отвечающий схемой `{ "embeddings": [[f32, ...], ...] }`. Пример — собственный сервер на FastAPI/Triton с моделью NomicEmbedTextV15.

## Быстрый старт

```bash
# 1. Поднять демон (background, daemonize).
gofer up

# 2. В корне проекта — зарегистрировать и запустить полную индексацию.
cd /path/to/your/project
gofer init
gofer start              # с watcher (по умолчанию)

# 3. Проверить состояние.
gofer status
gofer health

# 4. Подключить как MCP в клиенте — см. ниже.
```

Индексация выводит прогресс-бар по фазам `scan → parse → embed → write`. После завершения watcher следит за изменениями файлов и инкрементально обновляет индекс.

### Подключение MCP-клиента

В `.mcp.json` клиента (либо в его настройках):

```json
{
  "mcpServers": {
    "gofer": {
      "command": "/home/<user>/.cargo/bin/gofer",
      "args": ["mcp"]
    }
  }
}
```

`gofer mcp` автоматически поднимет демон, если он не запущен, и проксирует JSON-RPC между stdio клиента и сокетом демона. Рабочий каталог берётся из `cwd` процесса (можно переопределить флагом `--project-dir`).

## CLI

| Команда | Назначение |
|---|---|
| `gofer up` | Запустить демон в фоне. |
| `gofer down` | Остановить демон. |
| `gofer init` | Зарегистрировать текущий проект в демоне. |
| `gofer start [--watch]` | Активировать проект: полный sync + watcher. |
| `gofer sleep` | Деактивировать проект (индекс сохраняется, watcher останавливается). |
| `gofer watch` | `init` + `start --watch` + блокировка процесса. |
| `gofer mcp [--project-dir P]` | Запустить stdio-мост для MCP-клиента. |
| `gofer status` | Состояние демона и активных проектов. |
| `gofer health` | JSON со здоровьем демона. Код выхода ≠ 0, если не healthy. |
| `gofer search <q> [-l N]` | Гибридный поиск по индексу из CLI. |
| `gofer reindex [--force] [--path F]` | Инкрементальная или полная переиндексация (опционально одного файла). |
| `gofer logs [-n N] [-f] [--err]` | Хвост `~/.gofer/daemon.log` или `daemon.err`. |
| `gofer config [init\|path]` | Показать/инициализировать `.gofer/config.toml`. |
| `gofer install-lang <name>` | Скачать tree-sitter wasm-грамматику из lang-hub. |

## Конфигурация

Глобальный home: `~/.gofer/` — там лежат `daemon.sock`, `daemon.pid`, логи, реестр проектов, скачанные грамматики.

Проектный конфиг создаётся в `.gofer/config.toml`:

```bash
gofer config init    # положит дефолтный шаблон
gofer config path    # покажет, где он
gofer config         # выведет эффективные настройки
```

Ключевые секции (см. полный пример в `src/main.rs::DEFAULT_CONFIG`):

```toml
[server]
port = 10987

[indexer]
ignore = ["**/node_modules/**", "**/target/**", "**/.git/**", ...]
parallel_workers = 4

[embedding]
batch_size = 32
model = "NomicEmbedTextV15"
pool_size = 4
# external_url = "http://127.0.0.1:8080/embed/"  # переопределить эмбеддер

[reranker]
enabled = true
model_dir = ".gofer/data/models/reranker"

[domains]
rs_paths = []
py_paths = []
frontend_paths = []
ops_paths = []
shared_paths = []
```

## Метрики и наблюдаемость

- Демон поднимает Prometheus-эндпоинт на `127.0.0.1:9091/metrics`.
- Логи: `~/.gofer/daemon.log` (stdout) и `~/.gofer/daemon.err` (stderr), формат — `tracing` с JSON-фильтром.
- Уровень логов: флаг `--log-level debug|info|warn|error`.

## Архитектура одним абзацем

`src/main.rs` парсит CLI и для CLI-команд подключается к демону через `ipc::client::DaemonClient`. Демон (`src/daemon/`) живёт в отдельном процессе, держит `DaemonState` с реестром проектов, пулом эмбеддера, circuit breakers и менеджером языков. Indexer (`src/indexer/`) сканирует файлы, парсит их tree-sitter'ом, режет на чанки, эмбеддит и пишет в `SqliteStorage` + `LanceStorage`. MCP-инструменты (`src/daemon/handlers/`) маршрутизируются через `daemon/tools.rs::dispatch`. Полная схема — в [docs/architecture.md](docs/architecture.md).

## Документация

Главная точка входа — [docs/README.md](docs/README.md) (карта документации с маршрутами по ролям). Кратко:

**Начать пользоваться:**
- [docs/mcp-clients.md](docs/mcp-clients.md) — конфиги для Claude Code, Qoder, Cursor, Continue, Cline.
- [docs/embedder.md](docs/embedder.md) — контракт HTTP-эмбеддера, пример сервера.
- [docs/examples.md](docs/examples.md) — реальные сценарии и экономия токенов.
- [docs/benchmarks.md](docs/benchmarks.md) — цифры замеров: skeleton 76%, CAS 70–90%.

**Справочники:**
- [docs/tools-reference.md](docs/tools-reference.md) — 70+ MCP-инструментов + каталог lang-tools.
- [docs/config-reference.md](docs/config-reference.md) — полная схема `.gofer/config.toml`.
- [docs/errors.md](docs/errors.md) — JSON-RPC коды.
- [docs/models.md](docs/models.md) — структуры данных в индексе.
- [docs/storage-api.md](docs/storage-api.md) — публичный API `SqliteStorage`/`LanceStorage`.
- [docs/lang-hub.md](docs/lang-hub.md) — формат языковых пакетов.

**Архитектура и эксплуатация:**
- [docs/architecture.md](docs/architecture.md) — компоненты, pipeline, JSON-RPC, миграции, глоссарий, таблица лимитов.
- [docs/performance.md](docs/performance.md) — RAM/диск/latency, тюнинг.
- [docs/security.md](docs/security.md) — границы доверия, sandbox, IPC.
- [docs/logging.md](docs/logging.md) — формат tracing, фильтры, Loki/Vector.
- [docs/troubleshooting.md](docs/troubleshooting.md) — типичные проблемы.
- [docs/faq.md](docs/faq.md) — короткие ответы.
- [docs/roadmap.md](docs/roadmap.md) — статус фич.
- [docs/audit.md](docs/audit.md) — аудит инструментов и технические пробелы для агента.

**Разработка:**
- [docs/development.md](docs/development.md) — сборка, тесты, рецепты добавления.
- [CONTRIBUTING.md](CONTRIBUTING.md) — PR-гайд.

## Статус

Pet-проект, активная разработка. Phase 0 (Foundation) в основном выполнен, Phase 1–5 (runtime context, production intel, security, multi-version, sandbox) — в планах. API инструментов и схемы конфигов могут меняться без обещаний совместимости.
