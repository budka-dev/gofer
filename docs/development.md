# Разработка gofer

Этот документ — для тех, кто меняет код gofer'а: как собрать, как прогнать тесты, как добавить новый MCP-инструмент, новую миграцию и поддержку нового языка.

## Требования

- Rust toolchain — `1.93.0` подтверждён, минимум stable ≥ 1.90.
- Системные библиотеки: `git2` тянет libgit2 (либо сборку из исходников, либо системный pkg-config), `lancedb` — Apache Arrow C++, `reqwest` с rustls — OpenSSL не нужен.
- Опционально для разработки: `sqlx-cli` для миграций offline, `cargo-expand` для инспекции макросов, `cargo-clippy`.
- Запущенный HTTP-эмбеддер для интеграционных прогонов (см. `[embedding].external_url`).

## Сборка

```bash
cargo build                # dev (opt-level=1, инкрементальный)
cargo build --release      # релиз (lto=thin, strip)
cargo build --profile release-dev  # быстрый release-like с инкрементом
```

Установка в `~/.cargo/bin/`:

```bash
cargo install --path . --locked
```

Главный бинарь — `gofer`. Других crate в workspace нет.

## Запуск во время разработки

```bash
# Остановить запущенный демон (если стоит установленный gofer).
gofer down

# Поднять debug-сборку напрямую (без daemonize): команда `daemon` скрытая.
RUST_LOG=gofer=debug cargo run -- daemon &

# Или прогнать конкретную CLI-команду.
cargo run -- status
cargo run -- search "embedder pool"
```

При отладке MCP-моста удобно подключаться к dev-сборке: положи в свой `.mcp.json` абсолютный путь до `target/debug/gofer`.

## Тесты

```bash
cargo test                 # все unit + doc тесты внутри src/ под #[cfg(test)]
cargo test --release       # быстрее, если интересует только корректность
cargo test parser::        # фильтр по модулю
```

Все тесты живут как `#[cfg(test)] mod tests { ... }` внутри `src/`. Папка `tests/` в корне хранит **не интеграционные тесты**, а сравнительные отчёты (`tests/01_read_file_comparison.md` и т. д.) — документы про эффективность инструментов gofer vs native.

Тесты, требующие живых внешних зависимостей (эмбеддер, LSP-сервер), помечены `#[ignore]` или включены под фичу. Прогоняй точечно, когда подняты сервисы.

## Линтинг

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Pre-commit hook'и сейчас не настроены — соблюдай руками. См. `CLAUDE.md`, если есть в репо, для дополнительных правил.

## Структура кода (короткая шпаргалка)

| Хочешь добавить | Иди сюда |
|---|---|
| Новый MCP-инструмент в core-list | `src/daemon/handlers/<group>.rs` + ветка в `src/daemon/tools.rs::dispatch` + JSON-Schema в `core_tools_list` |
| Новый CLI-флаг/команду | `src/main.rs::Commands` + соответствующий `handle_*` |
| Новый JSON-RPC метод (`daemon/*`, не tool) | `src/ipc/server.rs::handle_request` + конкретный `handle_*` |
| Поддержку нового языка (парсинг) | новый файл в `src/languages/` (если нужна Vue-специфика) или добавить грамматику через lang-hub |
| Новую миграцию схемы | `migrations/018_*.sql` (см. ниже) |
| Новое поле в индексе | модель в `src/models/` + миграция + апдейт `indexer/pipeline.rs` |
| Новое поле в hot scoring | `src/scoring_index.rs::FileScoringData` + бамп `version` |

Полные таблицы маршрутизации — в [architecture.md](architecture.md).

## Добавление MCP-инструмента (рецепт)

1. **Имя.** Выбери стабильное snake_case-имя. Имя — публичный API; менять его потом дорого.
2. **Группа.** Подбери handler по семантике (`files`, `search`, `git`, …). Если не подходит ни одна — заведи новый файл в `src/daemon/handlers/` и зарегистрируй модуль в `handlers/mod.rs`.
3. **Сигнатура.** Добавь `pub async fn tool_my_new_thing(args: Value, ctx: &ToolContext) -> Result<Value>`. Парси аргументы через `args.get(...).and_then(|v| v.as_str())`. На отсутствующий required — `Err(GoferError::InvalidParams(...))`.
4. **Диспетч.** В `src/daemon/tools.rs::dispatch` добавь ветку:

   ```rust
   "my_new_thing" => mygroup::tool_my_new_thing(args, ctx).await,
   ```

5. **JSON-Schema.** В `core_tools_list()` добавь блок `json!({ "name": "my_new_thing", "description": "...", "inputSchema": { ... } })`. Описание должно быть **самодостаточным** — это то, что увидит модель в `tools/list`.
6. **Тесты.** Минимум — `#[tokio::test]` в самом handler с поднятым in-memory `SqliteStorage`. См. примеры в `handlers/files.rs`.
7. **Документация.** Допиши строку в [tools-reference.md](tools-reference.md) в соответствующую таблицу. Это часть PR, не «потом».

### Возврат данных

Возвращай `Value` — он попадёт в MCP-content как `text` (JSON-stringified). Старайся возвращать токен-оптимизированные структуры: плоские массивы строк или карты, группированные по файлу. Длинные тексты — режь `max_chars_per_op`-стилем, оставляя поля вроде `truncated: true`/`full_chars: N`.

## Добавление миграции SQLite

Файл `migrations/018_<name>.sql`. Имя — kebab-case, описательное. Внутри — обычный `CREATE TABLE IF NOT EXISTS`, `ALTER TABLE`, индексы. Транзакции `sqlx::migrate!` оборачивает сам.

Не правь существующие миграции — даже если они выглядят сломанными: индекс пользователей уже применил их с прежним хешем. Фиксы делай отдельной миграцией.

После добавления:

```bash
cargo sqlx prepare    # если используешь offline-режим (зависит от состояния .sqlx/)
cargo build           # подхватит migrations/ через include_dir
```

Обнови таблицу миграций в [architecture.md](architecture.md#миграции-sqlite).

## Структура папок проекта пользователя

При работе gofer создаёт в проекте:

```
.gofer/
├── config.toml           # необязательный, gofer config init
└── data/
    ├── index.sqlite      # метаданные
    ├── index.sqlite-wal  # SQLite WAL
    ├── lance/            # векторное хранилище
    └── models/           # rerank-модели (если включён [reranker])
```

И в `~/.gofer/`:

```
~/.gofer/
├── daemon.sock           # Unix socket
├── daemon.pid            # PID запущенного процесса
├── daemon.log            # stdout демона
├── daemon.err            # stderr демона
├── registry.sqlite       # реестр зарегистрированных проектов
├── langs/                # скачанные wasm-грамматики (gofer install-lang)
└── tools/                # бинарники LSP, если поднимаем сами
```

При сносе индекса достаточно удалить `<project>/.gofer/data/` и сделать `gofer reindex --force` (а ещё лучше `gofer init` заново — без удаления реестр продолжит ссылаться на проект).

## Отладка демона

- Логи: `gofer logs -n 200 -f` хвостит `~/.gofer/daemon.log`. Флаг `--err` переключает на `daemon.err`.
- Уровень логов: `gofer --log-level debug <cmd>` либо переменная `RUST_LOG=gofer=debug` (формат `env_logger`-совместимый).
- Метрики: `curl 127.0.0.1:9091/metrics` (Prometheus text exposition).
- Снапшот состояния: `gofer status` + `gofer health`.

Если сокет завис («Connection refused»): убей `~/.gofer/daemon.pid`, удали сам файл сокета (`rm ~/.gofer/daemon.sock`), снова `gofer up`. Граф `daemonize` не всегда корректно убирает сокет после SIGKILL.

## Pull request чек-лист

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` зелёные.
- Новые миграции пронумерованы по порядку, не правят существующие.
- Новый MCP-инструмент: handler + dispatch + schema + строка в `tools-reference.md`.
- Новые CLI-флаги: обновлён README.
- Не закоммичен `.sqlx/` мусор и `target/` (есть в `.gitignore`).
- Если ломаешь публичный API инструмента — отметь это в описании PR явно.
