# Справочник конфигурации

Все настройки gofer'а живут в двух местах:

1. **Проектный конфиг** — `.gofer/config.toml` (рядом с проектом).
2. **Глобальные настройки демона** — пока зашиты в коде. Изменяются только пересборкой; см. таблицу [Лимиты и константы](architecture.md#лимиты-и-константы) в архитектуре.

Создать дефолтный проектный конфиг:

```bash
gofer config init     # положит шаблон в .gofer/config.toml
gofer config path     # покажет путь
gofer config          # покажет эффективные значения
```

## Что реально парсится

`GoferConfig` (`src/indexer/watcher.rs::load_config`) парсит только две секции:

```toml
[indexer]
[embedding]
```

Всё, что в шаблоне `DEFAULT_CONFIG` помимо них (`[server]`, `[reranker]`, `[domains]`) — **зарезервированные** заголовки. Сейчас они либо никем не читаются, либо обслуживаются хардкод-дефолтами. Подробности — в конце документа.

## `[indexer]`

| Ключ | Тип | Дефолт | Что делает |
|---|---|---|---|
| `ignore` | `Vec<String>` | `[]` (но шаблон даёт большой список) | Дополнительные glob-паттерны для исключения файлов/директорий. Подключаются к `.gitignore` через crate `ignore`. Поверх gofer всегда исключает `.git`, `.gofer`, `.qoder`, `*.lock` — захардкожено в `build_gitignore` (`src/indexer/watcher.rs:101`). |
| `parallel_workers` | `Option<usize>` | вычисляется как `clamp(num_cpus/2, 4..=8)` | Сейчас параметр опционален и используется в нескольких местах, но фактическое число parser-воркеров в pipeline считается заново исходя из CPU — этот ключ задаёт мягкое предпочтение, не жёсткий лимит. |

Пример:

```toml
[indexer]
ignore = [
    "**/node_modules/**",
    "**/target/**",
    "**/.git/**",
    "**/dist/**",
    "**/build/**",
    "**/__pycache__/**",
    "**/coverage/**",
    "**/vendor/**",
    "**/.venv/**",
    "**/venv/**",
]
parallel_workers = 4
```

## `[embedding]`

Самая важная секция: gofer не считает эмбеддинги локально, ходит во внешний HTTP-сервис. Подробный контракт — в [embedder.md](embedder.md).

| Ключ | Тип | Дефолт | Что делает |
|---|---|---|---|
| `provider` | `String` | `"external"` | Метка провайдера. Сейчас единственное поведение — внешний HTTP, поэтому реально влияет только на логи. |
| `batch_size` | `usize` | `32` | Сколько чанков пакуется в один HTTP-запрос. Pipeline сам подстраивает реальный размер (`BATCH_CHUNK_SIZE_MIN=32`, `MAX=256`, `BASE=96`, timeout `50 ms`), но `batch_size` задаёт верхнюю границу для embedder-стейджа. |
| `pool_size` | `usize` | `4` | Число параллельных HTTP-клиентов. Внутри clamp до `1..=8` (`EmbedderPool::with_config`). При индексации поднимается до 4, после — скейлится вниз до 1. |
| `external_url` | `Option<String>` | `"http://127.0.0.1:8080/embed/"` | URL POST-эндпоинта эмбеддера. Если не задан, используется дефолт. |
| `external_api_key` | `Option<String>` | `None` | Если задан — добавляется в заголовок `x-api-key`. |
| `external_model` | `Option<String>` | `None` | Если задан — попадает в тело запроса как `"model"`. Используется как `model_name()` для `cache_version_key` (смена модели = инвалидация кеша эмбеддингов). |
| `dimensions` | `Option<usize>` | `1024` | Размерность эмбеддингов. Должна совпадать с тем, что отдаёт сервис, иначе LanceDB ругнётся на размер вектора. Не используется автодетект. |

Пример (как в шаблоне):

```toml
[embedding]
provider = "external"
batch_size = 32
pool_size = 4
external_url = "http://127.0.0.1:8080/embed/"
external_model = "nomic-embed-text-v1.5"
dimensions = 768
```

### Что происходит, если не указать `external_url`

Поднимется попытка POST на `http://127.0.0.1:8080/embed/`. Если там никого нет — `embedding_circuit` после 5 подряд ошибок размыкается на 30 с (`src/error_recovery.rs`). Поиск отвалится, всё остальное (read/grep/lsp) будет работать.

### Как сменить модель эмбеддера

1. Останови проект: `gofer sleep`.
2. Поменяй `external_url` / `external_model` / `dimensions` в `.gofer/config.toml`.
3. Снеси индекс: `rm -rf .gofer/data/`.
4. Запусти полный пересбор: `gofer start` (или `gofer reindex --force`).

Без переиндексации векторы в LanceDB останутся от старой модели — поиск будет давать мусор. Размерность нельзя поменять без полной пересборки.

## Глобальные настройки и `~/.gofer/`

Эти параметры в проектный конфиг не выносятся; меняются только пересборкой:

| Параметр | Значение | Где |
|---|---|---|
| Глобальный каталог демона | `~/.gofer/` | `src/main.rs::gofer_home` |
| Unix socket | `~/.gofer/daemon.sock` | `src/main.rs::socket_path` |
| Prometheus HTTP | `127.0.0.1:9091/metrics` | `src/main.rs::run_daemon` |
| Лог-файлы | `~/.gofer/daemon.log` / `daemon.err` | там же |
| Таймаут запуска демона | 30 с (300 × 100 мс) | `ensure_daemon_running` |
| Tokio worker threads (демон) | `max(num_cpus/2, 4)` | `run_daemon` |
| Tokio blocking threads (демон) | `num_cpus` | `run_daemon` |
| Максимальный размер файла для индекса | 2 МБ | `pipeline.rs::MAX_FILE_SIZE_BYTES` |
| Дебаунс watcher'а | 500 мс | `watcher.rs::start_watcher` |
| Cache: max записей | 100 000 | `pipeline.rs::CACHE_MAX_ENTRIES` |
| Cache: max возраст | 30 дней | `pipeline.rs::CACHE_MAX_AGE_DAYS` |
| Connection semaphore | 1024 | `daemon/state.rs:268` |
| ResourceLimits: max concurrent | 1024 | `resource_limits.rs:19` |
| Embedding circuit | 5 fail / 2 success / 30 с | `daemon/state.rs:248` |
| Vector circuit | 3 fail / 1 success / 10 с | `daemon/state.rs:255` |

## Уровень логов

CLI-флаг (глобальный):

```bash
gofer --log-level debug up
gofer --log-level warn mcp
```

Альтернатива через переменную:

```bash
RUST_LOG=gofer=debug,sqlx=warn gofer up
```

`tracing-subscriber` использует `EnvFilter`, синтаксис стандартный.

## Зарезервированные / не реализованные секции

Эти заголовки есть в шаблоне `gofer config init`, но **код их не читает**. Они оставлены как заготовка для будущих фич и сейчас никак не влияют на поведение.

### `[server]`

```toml
[server]
port = 10987
```

Не парсится. Демон слушает Unix-сокет, TCP-режим не реализован.

### `[reranker]`

```toml
[reranker]
enabled = true
model_dir = ".gofer/data/models/reranker"
```

Не парсится. В `daemon/state.rs:236` есть упоминание «loading the global embedder and reranker», но фактической реализации re-ranker'а в коде сейчас нет. Параметр `"rerank": true` в `search` существует, но handler его игнорирует.

### `[domains]`

```toml
[domains]
rs_paths = []
py_paths = []
frontend_paths = []
ops_paths = []
shared_paths = []
```

Не парсится `GoferConfig`. Используется хардкод-дефолт `DomainConfig::default_config()` из `src/indexer/domains.rs:42`:

- `rs_paths`: `backend/`, `server/`, `api/`, `src-rust/`, `src/`
- `py_paths`: `python/`, `app/`, `src/`
- `frontend_paths`: `frontend/`, `ui/`, `client/`, `src/`
- `ops_paths`: `ops/`, `deploy/`, `scripts/`
- `shared_paths`: `shared/`, `common/`, `models/`

Модуль помечен `#![allow(dead_code, unused_imports, unused_variables)]` — система доменов сейчас работает только на дефолтах.

## Где смотреть исходники

- `src/indexer/watcher.rs::load_config` — точка парсинга.
- `src/indexer/watcher.rs::GoferConfig` / `IndexerConfig` / `EmbeddingConfig` — структуры с `#[serde(default)]`.
- `src/main.rs::DEFAULT_CONFIG` — шаблон, который пишет `gofer config init`.
- `src/indexer/domains.rs::DomainConfig` — захардкоженные пути для доменов.
