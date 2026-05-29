# 21. Workspace State & TODO Notes

## Категория
Персистентное состояние

## Приоритет
🔴 **P1** (Очень полезно)

## Оценка полезности для AI
⭐⭐⭐⭐ (4/5)

## Описание
Сохранение состояния работы AI между сессиями и возможность оставлять TODO-заметки в коде.

## Проблема
AI начал большой рефакторинг, но сессия оборвалась. При следующем запуске AI "забывает", что он делал, и приходится начинать заново. Нет способа сохранить промежуточный прогресс.

## API

### save_workspace_state(state_id, metadata)
Сохранить текущее состояние работы.

**Параметры:**
- `state_id` (string) — уникальный ID состояния
- `metadata` (object) — произвольные данные о состоянии

**Возвращает:**
```json
{
  "state_id": "auth_refactor_20260225",
  "saved_at": "2026-02-25T10:30:00Z",
  "size": "45 KB"
}
```

**Пример metadata:**
```json
{
  "task": "Refactor authentication module",
  "progress": 60,
  "files_changed": ["src/auth.rs", "src/api.rs"],
  "next_steps": [
    "Update tests",
    "Add documentation",
    "Create PR"
  ],
  "context": "Extracted JWT verification into separate function",
  "active_transaction": "refactor_auth_tx"
}
```

**Пример использования:**
```
AI: (начал большой рефакторинг)
AI: patch_file(...)
AI: patch_file(...)
AI: save_workspace_state("auth_refactor_20260225", {
  task: "Refactor auth module",
  progress: 40,
  files_changed: ["src/auth.rs", "src/api.rs"],
  next_steps: ["Fix tests", "Update imports in 8 more files"]
})

(сессия оборвалась)
```

---

### load_workspace_state(state_id)
Загрузить сохранённое состояние.

**Параметры:**
- `state_id` (string) — ID состояния

**Возвращает:**
```json
{
  "state_id": "auth_refactor_20260225",
  "saved_at": "2026-02-25T10:30:00Z",
  "metadata": {
    "task": "Refactor authentication module",
    "progress": 40,
    "files_changed": ["src/auth.rs", "src/api.rs"],
    "next_steps": ["Fix tests", "Update imports in 8 more files"]
  }
}
```

**Пример использования:**
```
(новая сессия AI)
User: "Продолжи рефакторинг auth модуля"

AI: load_workspace_state("auth_refactor_20260225")
Result: {progress: 40, next_steps: ["Fix tests", "Update imports in 8 more files"]}

AI: "Продолжаю рефакторинг. Прогресс: 40%. Следующий шаг: фикс тестов."
AI: (продолжает с того места, где остановился)
```

---

### list_workspace_states()
Показать все сохранённые состояния.

**Возвращает:**
```json
{
  "states": [
    {
      "state_id": "auth_refactor_20260225",
      "task": "Refactor authentication module",
      "progress": 40,
      "saved_at": "2 hours ago",
      "size": "45 KB"
    },
    {
      "state_id": "api_migration_20260220",
      "task": "Migrate API to v2",
      "progress": 85,
      "saved_at": "5 days ago",
      "size": "120 KB"
    }
  ]
}
```

---

### delete_workspace_state(state_id)
Удалить сохранённое состояние.

**Возвращает:**
```json
{
  "state_id": "auth_refactor_20260225",
  "status": "deleted"
}
```

---

### add_todo_note(path, line, text, priority)
AI оставляет TODO-заметку в коде.

**Параметры:**
- `path` (string) — путь к файлу
- `line` (number) — строка для вставки TODO
- `text` (string) — текст заметки
- `priority` (string, optional) — `"low"` | `"medium"` | `"high"` (по умолчанию: medium)

**Возвращает:**
```json
{
  "path": "src/auth.rs",
  "line": 45,
  "todo_id": "todo_001",
  "text": "TODO(AI): Add rate limiting here"
}
```

**Вставляемый комментарий:**
```rust
// TODO(AI): Add rate limiting here [priority: high] [created: 2026-02-25]
```

**Пример:**
```
AI: patch_file("src/auth.rs", ...)
AI: add_todo_note("src/auth.rs", 45, "Add rate limiting here", priority="high")

Result: в src/auth.rs на строке 45 добавлен комментарий:
// TODO(AI): Add rate limiting here [priority: high] [created: 2026-02-25]
```

---

### list_todo_notes(filter)
Показать все TODO-заметки AI.

**Фильтры:**
- `path` — фильтр по файлу
- `priority` — `"low"` | `"medium"` | `"high"`
- `created_after` — дата

**Возвращает:**
```json
{
  "total": 5,
  "todos": [
    {
      "todo_id": "todo_001",
      "path": "src/auth.rs",
      "line": 45,
      "text": "Add rate limiting here",
      "priority": "high",
      "created_at": "2026-02-25T10:30:00Z"
    }
  ]
}
```

---

### resolve_todo_note(todo_id)
Пометить TODO как выполненный.

**Параметры:**
- `todo_id` (string)

**Возвращает:**
```json
{
  "todo_id": "todo_001",
  "status": "resolved",
  "resolved_at": "2026-02-25T14:30:00Z"
}
```

**Изменяет комментарий:**
```rust
// DONE(AI): Add rate limiting here [resolved: 2026-02-25]
```

## Примеры комплексного использования

### Сценарий 1: Долгий рефакторинг в несколько сессий
```
Session 1:
User: "Отрефактори auth модуль"
AI: (делает 40% работы)
AI: save_workspace_state("auth_refactor", {
  progress: 40,
  next_steps: ["Fix 8 test files", "Update imports"]
})
(сессия заканчивается)

Session 2 (на следующий день):
User: "Продолжи рефакторинг auth"
AI: load_workspace_state("auth_refactor")
AI: "Продолжаю. Прогресс: 40%. Следующий шаг: фикс тестов."
AI: (продолжает работу)
AI: save_workspace_state("auth_refactor", {progress: 70, ...})

Session 3:
AI: load_workspace_state("auth_refactor")
AI: (завершает рефакторинг)
AI: delete_workspace_state("auth_refactor")
AI: "Рефакторинг завершён!"
```

### Сценарий 2: TODO notes для отложенных задач
```
AI: (реализует основную фичу)
AI: patch_file("src/api.rs", ...)
AI: add_todo_note("src/api.rs", 100, "Add input validation", priority="high")
AI: add_todo_note("src/api.rs", 150, "Add logging", priority="medium")

(позже)
User: "Доделай все TODO"
AI: list_todo_notes({priority: "high"})
Result: 1 high-priority TODO

AI: (реализует валидацию)
AI: resolve_todo_note("todo_001")
AI: list_todo_notes()
Result: 1 medium-priority TODO осталось
```

## Преимущества

### 1. Долгие задачи
AI может работать над задачей несколько дней/недель с перерывами.

### 2. Контекст
AI не теряет контекст между сессиями.

### 3. Прозрачность
Пользователь видит прогресс и что осталось сделать.

### 4. Организация
TODO notes помогают AI структурировать отложенные задачи.

## Хранение состояния

### Структура
```
~/.gofer/indices/<project-uuid>/workspace_states/
  auth_refactor_20260225.json
  api_migration_20260220.json
  ...
```

### Формат state файла
```json
{
  "state_id": "auth_refactor_20260225",
  "created_at": "2026-02-25T10:00:00Z",
  "updated_at": "2026-02-25T10:30:00Z",
  "metadata": {
    "task": "...",
    "progress": 40,
    "files_changed": [...],
    "next_steps": [...],
    "context": "...",
    "active_transaction": "..."
  },
  "snapshots": {
    "transaction": {...},
    "uncommitted_changes": [...]
  }
}
```

## Интеграция с транзакциями

Если AI работает в транзакции и сохраняет state:
```rust
AI: begin_transaction("refactor")
AI: patch_file(...)
AI: save_workspace_state("refactor_state", {
  active_transaction: "refactor",
  transaction_operations: [...]
})

(session ends)

AI: load_workspace_state("refactor_state")
AI: (восстанавливает транзакцию)
AI: (продолжает добавлять операции)
AI: commit_transaction("refactor")
```

## Сложность реализации
Низкая (2-3 дня)
- Базовое сохранение/загрузка: очень низкая (1 день)
- TODO notes integration: низкая (1 день)
- Интеграция с транзакциями: низкая (1 день)

## Статус в gofer
❌ Отсутствует

## Зависимости
- JSON serialization
- filesystem API
- `begin_transaction` (для восстановления транзакций)

## Конфигурация

```toml
[workspace]
# Включить workspace states
enabled = true

# TTL для состояний (дни)
state_ttl_days = 30

# Максимальное количество состояний
max_states = 50

# Автоматическое сохранение состояния каждые N минут
auto_save_interval_minutes = 10

# TODO notes
todo_format = "TODO(AI)"  # формат комментария
```

## CLI для пользователя

```bash
# Показать сохранённые состояния
gofer workspace list

# Загрузить состояние
gofer workspace load <state-id>

# Удалить состояние
gofer workspace delete <state-id>

# Показать TODO notes
gofer todo list

# Пометить TODO как выполненный
gofer todo resolve <todo-id>
```

## Альтернативы
- AI запоминает через context (ограничено размером контекста)
- Git branches для состояний (сложно, загрязняет историю)
- Пользователь вручную трекает прогресс (неудобно)

## Связанные функции
- `begin_transaction` — восстановление транзакций
- `list_trash` — похожая концепция для удалённых файлов
