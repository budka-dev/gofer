# Troubleshooting

Список типичных сбоев и того, куда смотреть. Если симптома нет в таблице — открой `gofer logs -n 200 --err` и ищи слова `ERROR`/`WARN`.

## Демон не запускается

**`gofer up` молчит или валится с `Daemon failed to start within 30 seconds`.**

1. Посмотри `~/.gofer/daemon.err` — daemonize дублирует туда stderr процесса.
2. Если файл сокета остался от прошлого процесса:
   ```bash
   rm ~/.gofer/daemon.sock ~/.gofer/daemon.pid
   gofer up
   ```
3. Если порт 9091 занят (Prometheus-метрики) — демон стартует, но `serve_metrics` пишет ошибку в лог. Сам демон при этом живой. Освободи порт или допили `metrics_http::serve_metrics` под env.
4. Если падает на старте на миграции — `rm -rf ~/.gofer/registry.sqlite` (только для dev-окружения, это снесёт реестр проектов).

**`Address already in use` на сокете.**

`daemonize` иногда не убирает сокет при SIGKILL. Команды выше (удалить `.sock` и `.pid`) лечат.

## `gofer mcp` отваливается у клиента

**Симптом:** клиент (Claude Code, Qoder) пишет «MCP server crashed», тулзы недоступны.

1. Запусти `gofer mcp` в терминале вручную — увидишь, что он печатает в stderr. Часто причина — `ensure_daemon_running()` не смог поднять демон (см. выше).
2. Проверь путь в `.mcp.json` — он абсолютный? `~` клиент не разворачивает.
3. Проверь, что бинарь не из старой версии: `gofer --version`. Если ставил из `cargo install`, новая сборка приехала в `~/.cargo/bin/`, а в `.mcp.json` мог быть прописан `target/debug/gofer`.

## Эмбеддер недоступен

**Симптом:** `gofer logs` забит `Failed to embed query: connection refused` или `Circuit breaker open`.

1. Подними HTTP-эмбеддер на адресе из `.gofer/config.toml::[embedding].external_url` (по умолчанию `http://127.0.0.1:8080/embed/`). Проверь руками:
   ```bash
   curl -X POST http://127.0.0.1:8080/embed/ -d '{"texts":["test"]}' -H 'content-type: application/json'
   ```
2. После пяти подряд ошибок `embedding_circuit` размыкается на 30 с (`src/error_recovery.rs`, конфиг в `daemon/state.rs:248`). После восстановления сервиса жди до 30 с, пока цепь сама не перейдёт в HalfOpen и не закроется.
3. Поиск без эмбеддера: `search` отвалится. `grep`, `find_files`, `read_file`, `get_symbols`, `structural_search`, git-инструменты — работают, они не ходят в эмбеддер.

## Индекс «битый» или неполный

**Симптом:** `get_index_status` показывает completeness < 1.0, `search` пропускает свежие файлы, watcher не реагирует.

1. Сначала диагностика:
   ```bash
   gofer mcp <args>   # либо вызови tools через клиента:
   #   - get_index_status
   #   - validate_index
   ```
   `validate_index` помечает расхождения между диском и SQLite.
2. Инкремент:
   ```bash
   gofer reindex
   ```
3. Полная переиндексация:
   ```bash
   gofer reindex --force
   ```
4. Жёсткий снос (последняя инстанция):
   ```bash
   rm -rf .gofer/data/
   gofer reindex --force
   ```

Файлы > 2 МБ (`MAX_FILE_SIZE_BYTES` в `indexer/pipeline.rs`) пропускаются молча, это by design. Если их надо индексировать — патчь константу.

## Watcher не видит изменения

**Симптом:** меняешь файл, `search` отдаёт старое.

1. Дебаунс — 500 мс. Если печатаешь быстро, события собираются в один пакет — это нормально.
2. Проверь `[indexer].ignore` в `.gofer/config.toml` — твой каталог не попал под игнор?
3. На Linux лимит `fs.inotify.max_user_watches` мог исчерпаться. Большие проекты иногда упираются:
   ```bash
   sysctl fs.inotify.max_user_watches
   sudo sysctl -w fs.inotify.max_user_watches=524288
   ```
   Долговременно — в `/etc/sysctl.d/`.
4. Если watcher не стартовал вообще, в `daemon.log` будет `Failed to watch dir: <path>: <error>`. Проверь права.

## `search` отдаёт ерунду / низкие score

1. Проверь, что эмбеддер тот же, что был при индексации. Смена модели = инвалидация кеша через `cache_version_key`. Если переехал на другой эмбеддер — сделай `gofer reindex --force`.
2. Подыми `min_score` (по умолчанию 0.0). Например, `min_score: 0.4` отсечёт мусор.
3. Включи `include_scores: true` — посмотри распределение. Если всё в районе 0.1–0.2 — это симптом разной модели эмбеддера vs та, которой считали индекс.
4. Re-ranker отключён? Включи `[reranker].enabled = true` в `.gofer/config.toml`.

## `tools/call` отвечает `-32601 Method not found`

Имя инструмента ушло мимо `dispatch`. Возможные причины:

- Опечатка в имени (`get_symbols` ≠ `get_symbol`).
- Инструмент существует в handler, но не зарегистрирован в `dispatch` — проверь `src/daemon/tools.rs::dispatch`.
- Старый MCP-клиент кеширует `tools/list`. Перезапусти клиент или вызови `tools/list` снова.

## `tools/call` отвечает `-32602 Invalid params`

В сообщении ошибки есть какой именно параметр пропущен. Чаще всего:

- Забыт `project_path` (для CLI-вызова через сокет — он не инжектится bridge'м, нужен явно).
- Required-параметр пропущен (см. `tools-reference.md`).
- Тип не совпадает (`limit` строкой вместо integer).

## Высокая память у демона

1. `EmbedderPool` после индексации скейлится до 1 (`pipeline.rs:254`). Если в логе `Failed to scale down embedder pool` — пул застрял на 4.
2. LanceDB фрагменты: `lance.compact()` вызывается в конце pipeline, но при частых мелких инкрементах фрагменты копятся. Раз в N часов `gofer reindex` или ручной compact полезен.
3. SQLite WAL может разрастаться. `gofer down` нормально его свернёт.
4. jemalloc по умолчанию агрессивен на возврат RSS — это норм. Метрика «процесс ест 2 ГБ» в `top` обычно держится включая зарезервированную, не использованную память.

## Не подхватывается язык

**Симптом:** `gofer install-lang <name>` падает или язык не используется при индексации.

1. Имя должно совпадать с папкой в lang-hub репозитории. Проверь там перечень.
2. После установки грамматика лежит в `~/.gofer/langs/<name>/`. Если файл `.wasm` есть, но парсер не подключается — посмотри `daemon.log` на `wasm ABI version mismatch`. Это значит, что версия грамматики не совпадает с tree-sitter в gofer. См. коммит `d99d62e` про мягкую обработку этой ошибки.
3. Для собственной грамматики положи `language.wasm` руками в `~/.gofer/langs/<name>/` и перезапусти демона.

## Когда писать issue

Если симптом не покрыт и логи не помогают:

1. Включи `RUST_LOG=gofer=debug`, повтори, собери `~/.gofer/daemon.log` за последние 5 минут.
2. Приложи `gofer health`, `gofer status`, `gofer config`.
3. Опиши версию: `gofer --version`, OS, Rust toolchain (`rustc --version`).
4. Если завязано на индексе — оцени размер проекта в файлах и LOC.
