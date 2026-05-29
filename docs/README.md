# Документация gofer

Главная карта документов. Чтобы понять, куда смотреть, выбери роль:

## Я хочу попробовать gofer (пользователь)

1. [Корневой README](../README.md) — установка и быстрый старт.
2. [embedder.md](embedder.md) — без HTTP-эмбеддера ничего не заработает; начни с него.
3. [mcp-clients.md](mcp-clients.md) — конфиги для Claude Code, Qoder, Cursor, Continue, Cline.
4. [examples.md](examples.md) — типичные сценарии и экономия токенов.
5. [benchmarks.md](benchmarks.md) — цифры экономии: skeleton 76%, CAS-буфер до 90%.
6. [faq.md](faq.md) — короткие ответы на частые вопросы.
7. [troubleshooting.md](troubleshooting.md) — когда что-то пошло не так.

## Я подключаю gofer как MCP-сервер (интегратор)

1. [mcp-clients.md](mcp-clients.md) — конфиги для всех популярных клиентов в одном месте.
2. [tools-reference.md](tools-reference.md) — полный список доступных инструментов и их параметров.
3. [errors.md](errors.md) — JSON-RPC коды и как их интерпретировать.
4. [config-reference.md](config-reference.md) — что положить в `.gofer/config.toml`.
5. [security.md](security.md) — границы доверия, sandbox, утечки через эмбеддер.
6. [architecture.md](architecture.md) → разделы «Sequence: tools/call», «Маршрутизация MCP-инструментов».

## Я меняю код gofer'а (контрибьютор)

1. [../CONTRIBUTING.md](../CONTRIBUTING.md) — короткий гайд для PR.
2. [architecture.md](architecture.md) — общая картина: слои, демон, pipeline, IPC.
3. [development.md](development.md) — сборка, тесты, рецепты добавления инструмента/миграции.
4. [models.md](models.md) — структуры данных в индексе.
5. [storage-api.md](storage-api.md) — публичный API `SqliteStorage`/`LanceStorage`.
6. [errors.md](errors.md) — иерархия ошибок, как их возвращать.
7. [tools-reference.md](tools-reference.md) — чтобы не наплодить дубль уже существующего инструмента.
8. [audit.md](audit.md) — что лишнее, что укрупнить, чего критически не хватает для агента (с приоритетами P0/P1/P2).
9. [lang-hub.md](lang-hub.md) — формат языкового пакета, если добавляешь поддержку нового языка.
10. [roadmap.md](roadmap.md) — что реализовано, что в планах.

## Я отлаживаю / эксплуатирую (оператор)

1. [troubleshooting.md](troubleshooting.md) — типовые симптомы и решения.
2. [performance.md](performance.md) — RAM, диск, latency, тюнинг.
3. [logging.md](logging.md) — формат, фильтры, интеграция с Loki/Vector.
4. [security.md](security.md) — что уходит наружу, IPC permissions, audit log.
5. [architecture.md](architecture.md) → разделы «Лимиты и константы», «Метрики и наблюдаемость», «Graceful shutdown».
6. [config-reference.md](config-reference.md) → раздел «Глобальные настройки» и таблица захардкоженных параметров.

## Полный список документов

### Основа

| Файл | О чём |
|---|---|
| [examples.md](examples.md) | Сценарии использования: подключение, поиск, патч, batch, CAS-буфер, lang-tools. |
| [benchmarks.md](benchmarks.md) | Сводная таблица замеров vs native: skeleton 76%, batch 3–5×, CAS 70–90%. |
| [mcp-clients.md](mcp-clients.md) | Конфиги для Claude Code, Qoder, Cursor, Continue, Cline. |
| [faq.md](faq.md) | Короткие частые вопросы. |
| [troubleshooting.md](troubleshooting.md) | Демон не стартует, эмбеддер недоступен, индекс битый, watcher не реагирует, LSP молчит. |

### Архитектура и внутренности

| Файл | О чём |
|---|---|
| [architecture.md](architecture.md) | Карта процессов, слои кода, pipeline, IPC, JSON-RPC, MCP resources, миграции, watcher, sandbox, sequence, глоссарий, таблица лимитов. |
| [tools-reference.md](tools-reference.md) | Полный каталог MCP-инструментов (core + транзакции + lang-tools). |
| [models.md](models.md) | Структуры данных в индексе: `Symbol`, `Chunk`, `SymbolReference`, `SymbolKind`. |
| [storage-api.md](storage-api.md) | Публичный API `SqliteStorage`/`LanceStorage` по группам. |
| [errors.md](errors.md) | Иерархия `GoferError` и JSON-RPC коды. |
| [logging.md](logging.md) | Формат tracing, `RUST_LOG`, интеграция с Loki/Vector. |

### Конфигурация и расширение

| Файл | О чём |
|---|---|
| [config-reference.md](config-reference.md) | Полная схема `.gofer/config.toml`. |
| [embedder.md](embedder.md) | HTTP-контракт эмбеддера, форматы ответа, минимальный сервер. |
| [lang-hub.md](lang-hub.md) | Формат языковых пакетов, `gofer install-lang`. |

### Эксплуатация

| Файл | О чём |
|---|---|
| [performance.md](performance.md) | RAM/диск/CPU, capacity planning, тюнинг. |
| [security.md](security.md) | Модель безопасности: что индексируется, sandbox, IPC, эмбеддер. |
| [roadmap.md](roadmap.md) | Статус фич по фазам. |
| [audit.md](audit.md) | Аудит инструментов: дубли, заглушки, критические пробелы для технического copilot'а (P0/P1/P2). |

### Разработка

| Файл | О чём |
|---|---|
| [development.md](development.md) | Сборка, тесты, рецепты добавления инструмента/миграции, отладка демона, PR-чеклист. |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Короткий гайд для внешних PR. |
| [../tests/README.md](../tests/README.md) | Индекс сравнительных отчётов инструментов vs native. |

## Что НЕ задокументировано

После всех проходов остался один честный пробел:

- **Точные RPS/latency бенчмарки для крупных монорепо** — есть только порядковые оценки в [performance.md](performance.md) и сравнения на pet-проекте в [benchmarks.md](benchmarks.md). Реальные замеры на проектах >500K LOC никто не делал.

Если документация чего-то ещё умалчивает — открывай issue или PR. См. [../CONTRIBUTING.md](../CONTRIBUTING.md).
