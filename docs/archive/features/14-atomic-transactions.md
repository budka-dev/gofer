# 14. Atomic Transactions

## Категория
Транзакционные операции

## Приоритет
🔥 **P0** (Критично)

## Оценка полезности для AI
⭐⭐⭐⭐⭐ (5/5)

## Описание
Транзакционная модель для файловых операций: все изменения применяются атомарно (всё или ничего).

## Проблема
AI часто меняет несколько файлов одновременно (например, переименовывает функцию → обновляет импорты в 10 файлах). Если AI ошибётся в 8-м файле, предыдущие 7 уже испорчены. Нужен механизм отката всех изменений при ошибке.

## Концепция
Как в базах данных: BEGIN → операции → COMMIT (всё применяется) или ROLLBACK (всё откатывается).

## API

### begin_transaction(transaction_id)
Начать транзакцию. Все изменения буферизуются в памяти.

```
begin_transaction(transaction_id) -> TransactionHandle
```

**Возвращает:**
```json
{
  "transaction_id": "refactor_auth_20260225",
  "status": "active",
  "started_at": "2026-02-25T10:30:00Z"
}
```

---

### add_operation(transaction_id, operation)
Добавить операцию в транзакцию.

```
add_operation(transaction_id, operation) -> OperationResult
```

**Поддерживаемые операции:**
- `patch_file`
- `write_file`
- `delete_safe`
- `move_file_or_directory`
- `append_to_file`

**Параметры:**
```json
{
  "transaction_id": "refactor_auth_20260225",
  "operation": {
    "type": "patch_file",
    "params": {
      "path": "src/auth.rs",
      "search_string": "fn login(",
      "replace_string": "fn authenticate("
    }
  }
}
```

**Возвращает:**
```json
{
  "operation_id": "op_001",
  "status": "staged",
  "validation": {
    "syntax_check": "passed",
    "conflicts": []
  }
}
```

---

### commit_transaction(transaction_id)
Атомарно применить ВСЕ операции транзакции.

```
commit_transaction(transaction_id) -> CommitResult
```

**Логика:**
1. Проверить все операции на конфликты
2. Создать backup текущего состояния (snapshot)
3. Применить все операции последовательно
4. Если хотя бы одна ошибка → откатить к snapshot
5. Если всё ОК → удалить snapshot, вернуть success

**Возвращает:**
```json
{
  "transaction_id": "refactor_auth_20260225",
  "status": "committed",
  "operations_applied": 10,
  "files_changed": ["src/auth.rs", "src/api.rs", "src/tests.rs", ...],
  "committed_at": "2026-02-25T10:35:00Z"
}
```

---

### rollback_transaction(transaction_id)
Отменить все изменения в транзакции.

```
rollback_transaction(transaction_id) -> RollbackResult
```

**Возвращает:**
```json
{
  "transaction_id": "refactor_auth_20260225",
  "status": "rolled_back",
  "operations_discarded": 10
}
```

---

### list_transactions()
Показать активные транзакции.

```
list_transactions() -> TransactionList
```

**Возвращает:**
```json
{
  "transactions": [
    {
      "transaction_id": "refactor_auth_20260225",
      "status": "active",
      "operations_count": 5,
      "started_at": "10 minutes ago"
    }
  ]
}
```

## Примеры использования

### Пример 1: Переименование функции во всём проекте
```
AI: begin_transaction("rename_login_to_auth")

AI: add_operation("rename_login_to_auth", {
  type: "patch_file",
  params: {path: "src/auth.rs", search: "fn login(", replace: "fn authenticate("}
})

AI: add_operation("rename_login_to_auth", {
  type: "patch_file",
  params: {path: "src/api.rs", search: "auth::login", replace: "auth::authenticate"}
})

... (8 операций на 8 файлов)

AI: commit_transaction("rename_login_to_auth")
Result: ВСЕ 10 файлов изменены атомарно
```

### Пример 2: Ошибка в середине — откат всего
```
AI: begin_transaction("big_refactor")
AI: add_operation("big_refactor", {type: "patch_file", params: {...}})  # OK
AI: add_operation("big_refactor", {type: "patch_file", params: {...}})  # OK
AI: add_operation("big_refactor", {type: "patch_file", params: {...}})  # ОШИБКА синтаксиса

AI: commit_transaction("big_refactor")
Result: {
  status: "failed",
  error: "Syntax error in operation 3",
  action: "All changes rolled back automatically"
}
# Все файлы остались в исходном состоянии!
```

### Пример 3: Явный откат
```
AI: begin_transaction("experimental_changes")
AI: add_operation(...)
AI: add_operation(...)
AI: (понимает, что подход неверный)
AI: rollback_transaction("experimental_changes")
Result: все изменения отменены
```

## Преимущества

### 1. Безопасность
AI не может "наполовину испортить" проект. Либо всё работает, либо ничего не изменилось.

### 2. Уверенность AI
AI может смело делать сложные рефакторинги, зная что есть откат.

### 3. Отладка
Если транзакция фейлится, AI видит конкретную операцию, которая вызвала ошибку.

### 4. Производительность
Все проверки (syntax, conflicts) делаются **до** применения изменений.

## Интеграция с другими фичами

### delete_safe
```
AI: add_operation(tx_id, {type: "delete_safe", params: {path: "..."}})
```
Файл удаляется (→ trash) только при commit. При rollback — остаётся на месте.

### verify_patch
Каждая операция автоматически проверяется через компилятор перед commit.

## Архитектура

### Хранение state транзакции
```rust
struct Transaction {
    id: String,
    operations: Vec<Operation>,
    snapshot: Option<Snapshot>,  // Backup для отката
    status: TransactionStatus,
}

enum Operation {
    PatchFile { path, search, replace },
    WriteFile { path, content },
    DeleteSafe { path, reason },
    MoveFile { source, dest },
}
```

### Snapshot механизм
При commit создаётся временный snapshot:
```
~/.gofer/indices/<project-uuid>/snapshots/<transaction-id>/
  file1.rs.backup
  file2.rs.backup
  ...
```

Если commit успешен → snapshot удаляется.  
Если ошибка → snapshot используется для восстановления.

## Сложность реализации
Средняя (5-7 дней)
- Базовая транзакция (begin/commit/rollback): 2 дня
- Snapshot механизм: 2 дня
- Интеграция с file operations: 2 дня
- Валидация и error handling: 1 день

## Статус в gofer
❌ Отсутствует

## Зависимости
- Все file operations (`patch_file`, `write_file`, и т.д.)
- Snapshot storage (filesystem)
- `verify_patch` для валидации

## Альтернативы
- Git branches для каждого изменения (сложно)
- Ручной откат через trash (неполно)
- Нет отката (опасно)

## Связанные функции
- Все file operations (работают через транзакции)
- `delete_safe` — интеграция с trash
- `verify_patch` — валидация операций
