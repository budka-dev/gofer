# Подключение MCP-клиентов

Конфиги для всех популярных MCP-клиентов. Везде одинаковая логика: `command` = `gofer`, `args` = `["mcp"]`, опционально `--project-dir`.

Предполагается, что демон уже запущен (`gofer up`) и проект активирован (`gofer init && gofer start`). `gofer mcp` сам поднимет демон, если что — это не критично.

## Claude Code

### Через settings.json

В `~/.claude.json` или `~/.config/claude/claude.json`:

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

Через CLI:

```bash
claude mcp add gofer --command /home/<user>/.cargo/bin/gofer --args mcp
```

### Per-project

В `.mcp.json` рядом с проектом:

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

Claude Code прочитает локальный конфиг при запуске из этого каталога. cwd процесса `gofer mcp` будет корнем проекта — `gofer` сам поймёт, какой проект использовать.

### С явным project-dir

Если запускаешь Claude Code из другого каталога, но индексируешь конкретный проект:

```json
{
  "mcpServers": {
    "gofer": {
      "command": "/home/<user>/.cargo/bin/gofer",
      "args": ["mcp", "--project-dir", "/abs/path/to/project"]
    }
  }
}
```

После настройки инструменты появятся под префиксом `mcp__gofer__*`. Перезапусти Claude Code после правки конфига.

## Qoder

В Qoder откройте Settings → MCP Servers и добавьте:

```json
{
  "gofer": {
    "command": "/home/<user>/.cargo/bin/gofer",
    "args": ["mcp"]
  }
}
```

Либо через UI: «Add MCP Server» → Command: `gofer`, Args: `mcp`.

## Cursor

Cursor использует тот же формат `.mcp.json`:

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

Положи в `~/.cursor/mcp.json` (глобально) или в `.cursor/mcp.json` в корне проекта.

После — Cmd+Shift+P → Reload MCP Servers.

## Continue (VS Code / JetBrains)

В `~/.continue/config.json`:

```json
{
  "experimental": {
    "modelContextProtocolServers": [
      {
        "transport": {
          "type": "stdio",
          "command": "/home/<user>/.cargo/bin/gofer",
          "args": ["mcp"]
        },
        "name": "gofer"
      }
    ]
  }
}
```

Префикс инструментов в Continue — `gofer__*`.

## Cline (Claude Dev для VS Code)

В `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` (Linux) или эквивалентный путь для Mac/Windows:

```json
{
  "mcpServers": {
    "gofer": {
      "command": "/home/<user>/.cargo/bin/gofer",
      "args": ["mcp"],
      "disabled": false,
      "autoApprove": []
    }
  }
}
```

`autoApprove` — список имён инструментов, которые Cline вызывает без подтверждения. Безопасный набор для начала:

```json
"autoApprove": [
  "read_file", "skeleton", "search", "get_symbols",
  "find_files", "grep", "project_tree", "get_index_status"
]
```

Все инструменты gofer — read-only, поэтому `autoApprove` можно расширить любым из них без риска нежелательных изменений на диске.

## Произвольный MCP-клиент

Любой клиент с поддержкой stdio MCP подключается одинаково:

- **Command:** абсолютный путь к `gofer` (узнать: `which gofer`).
- **Args:** `["mcp"]` или `["mcp", "--project-dir", "<path>"]`.
- **Env:** опционально `RUST_LOG=gofer=debug` для отладочного лога.

Транспорт — stdio JSON-RPC 2.0. Протокол — стандартный MCP, никаких vendor-расширений.

## Без UI-клиента: голый JSON-RPC

Если хочется поскриптовать без MCP-клиента:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"project_path":"'$PWD'"}}' \
  | nc -U ~/.gofer/daemon.sock
```

`gofer mcp` — это просто bridge stdio↔socket. Можешь обращаться к сокету напрямую любым JSON-RPC клиентом.

## Несколько проектов в одном клиенте

gofer одной командой обслуживает несколько проектов: достаточно зарегистрировать их (`gofer init && gofer start`) в каждом каталоге. MCP-клиент при подключении просто берёт текущий cwd как корень проекта.

Если хочется одновременно работать с несколькими проектами через один клиент — можно настроить несколько MCP-инстансов с разными `--project-dir`:

```json
{
  "mcpServers": {
    "gofer-backend": {
      "command": "/home/<user>/.cargo/bin/gofer",
      "args": ["mcp", "--project-dir", "/work/backend"]
    },
    "gofer-frontend": {
      "command": "/home/<user>/.cargo/bin/gofer",
      "args": ["mcp", "--project-dir", "/work/frontend"]
    }
  }
}
```

Демон один, проекта два — каждый bridge подключается к своему `ProjectState`.

## Проверка подключения

Если клиент молчит:

1. Убедись, что бинарь работает: `gofer mcp --help`.
2. Запусти руками: `gofer mcp` — в stderr увидишь, что bridge стартует и пытается подключиться к демону. При успехе будет тишина (ждёт JSON на stdin).
3. Проверь, что демон жив: `gofer status`.
4. Логи демона: `gofer logs -n 50`.

Если клиент пишет «MCP server crashed» — почти всегда либо неправильный путь, либо `~` не развернулся (нужен абсолютный путь), либо демон не стартует. См. [troubleshooting.md::`gofer mcp` отваливается у клиента](troubleshooting.md#gofer-mcp-отваливается-у-клиента).

## Безопасность

При интеграции с MCP-клиентом учти [security.md](security.md):

- gofer — read-only: не изменяет файлы, не выполняет код. Все инструменты безопасно добавлять в `autoApprove`.
- Единственный сетевой риск: содержимое файлов уходит в HTTP-эмбеддер при индексации. Настрой локальный эмбеддер, если это критично (см. [security.md](security.md)).
