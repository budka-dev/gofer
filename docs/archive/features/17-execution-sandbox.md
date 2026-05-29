# 17. Execution Sandbox

## Категория
Проверка и выполнение кода

## Приоритет
🔥 **P0** (Критично)

## Оценка полезности для AI
⭐⭐⭐⭐⭐ (5/5)

## Описание
Безопасное выполнение кода в изолированном окружении с получением результата или ошибки.

## Проблема
AI генерирует функцию, но не может проверить, работает ли она. Нужно либо ждать пользователя, либо полагаться только на статический анализ. Это превращает AI из "генератора кода" в "программиста, который проверяет свою работу".

## API

### execute_function(path, function_name, args, timeout)
Запустить конкретную функцию в изолированном окружении.

**Параметры:**
- `path` (string) — путь к файлу
- `function_name` (string) — имя функции
- `args` (array) — аргументы для функции (JSON)
- `timeout` (number, optional) — таймаут в секундах (по умолчанию: 5)

**Возвращает:**
```json
{
  "status": "success",
  "result": [2, 3, 5, 7, 11, 13, 17, 19, 23, 29],
  "execution_time_ms": 12,
  "stdout": "",
  "stderr": ""
}
```

**Примеры:**

#### Rust
```
AI: patch_file("src/math.rs", ..., replace="pub fn calculate_primes(n: usize) -> Vec<usize> { ... }")
AI: execute_function("src/math.rs", "calculate_primes", args=[30], timeout=5)

Result: {
  status: "success",
  result: [2, 3, 5, 7, 11, 13, 17, 19, 23, 29]
}

AI: "Функция работает корректно!"
```

#### Python
```
AI: patch_file("utils.py", ..., replace="def fibonacci(n): ...")
AI: execute_function("utils.py", "fibonacci", args=[10])

Result: {
  status: "success",
  result: [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]
}
```

#### Ошибка выполнения
```
AI: execute_function("src/buggy.rs", "divide", args=[10, 0])

Result: {
  status: "error",
  error_type: "runtime_error",
  message: "attempt to divide by zero",
  stderr: "thread 'main' panicked at 'attempt to divide by zero'"
}

AI: "Обнаружена ошибка деления на ноль, добавляю проверку..."
```

---

### run_test(path, test_name)
Запустить конкретный тест.

**Параметры:**
- `path` (string) — путь к файлу
- `test_name` (string, optional) — имя теста (если не указан — все тесты в файле)

**Возвращает:**
```json
{
  "status": "passed",
  "tests_run": 5,
  "tests_passed": 5,
  "tests_failed": 0,
  "execution_time_ms": 120,
  "details": [
    {
      "name": "test_authenticate_success",
      "status": "passed",
      "duration_ms": 24
    }
  ]
}
```

**Пример:**
```
AI: patch_file("src/auth.rs", ...)
AI: run_test("src/auth.rs", "test_authenticate_success")

Result: {status: "passed"}

AI: "Тест прошёл успешно!"
```

---

### run_all_tests(filter)
Запустить все тесты проекта или с фильтром.

**Параметры:**
- `filter` (string, optional) — фильтр по имени теста (regex)

**Возвращает:**
```json
{
  "status": "partial_failure",
  "total_tests": 150,
  "passed": 148,
  "failed": 2,
  "skipped": 0,
  "execution_time_ms": 5420,
  "failures": [
    {
      "test": "test_auth_with_invalid_token",
      "file": "src/auth.rs",
      "line": 42,
      "error": "assertion failed: token.is_valid()"
    }
  ]
}
```

**Пример:**
```
AI: (завершил рефакторинг)
AI: run_all_tests()

Result: 2 failed tests

AI: (анализирует failures)
AI: patch_file("src/auth.rs", ...) # фиксит ошибки
AI: run_all_tests()

Result: all tests passed
```

---

### benchmark(path, function_name, iterations)
Замерить производительность функции.

**Параметры:**
- `path` (string) — путь к файлу
- `function_name` (string) — имя функции
- `iterations` (number, optional) — количество итераций (по умолчанию: 1000)

**Возвращает:**
```json
{
  "function": "calculate_primes",
  "iterations": 1000,
  "avg_time_ms": 0.45,
  "min_time_ms": 0.42,
  "max_time_ms": 1.2,
  "std_dev_ms": 0.08
}
```

**Пример:**
```
AI: benchmark("src/math.rs", "calculate_primes", iterations=1000)

Result: avg 0.45ms per call

AI: "Оптимизирую алгоритм..."
AI: patch_file("src/math.rs", ...)
AI: benchmark("src/math.rs", "calculate_primes", iterations=1000)

Result: avg 0.12ms per call

AI: "Производительность улучшена в 3.75 раза!"
```

## Изоляция и безопасность

### Варианты изоляции

#### 1. WASM Sandbox (рекомендуется для Rust)
```
- Компиляция в WASM
- Выполнение в изолированном WASI runtime
- Нет доступа к filesystem/network
- Быстро и безопасно
```

#### 2. Docker Container
```
- Полная изоляция ОС
- Поддержка любых языков
- Медленнее, но универсально
```

#### 3. VM (для Python/JS)
```
- Python: RestrictedPython
- JavaScript: isolated-vm
- Ограниченный доступ к API
```

### Лимиты ресурсов
```toml
[execution]
max_memory_mb = 512
max_cpu_time_seconds = 5
max_file_operations = 0  # запретить file I/O
max_network_requests = 0  # запретить network
```

## Примеры комплексного использования

### Сценарий: TDD — AI пишет тест и реализацию
```
User: "Напиши функцию для валидации email"

AI: patch_file("src/validation.rs", ..., replace="
  #[test]
  fn test_validate_email() {
    assert!(validate_email(\"user@example.com\"));
    assert!(!validate_email(\"invalid\"));
  }
")

AI: run_test("src/validation.rs", "test_validate_email")
Result: {status: "failed", error: "function validate_email not found"}

AI: patch_file("src/validation.rs", ..., replace="
  pub fn validate_email(email: &str) -> bool {
    email.contains('@') && email.contains('.')
  }
")

AI: run_test("src/validation.rs", "test_validate_email")
Result: {status: "passed"}

AI: "Функция реализована и протестирована!"
```

### Сценарий: AI находит и фиксит баг через выполнение
```
User: "Функция fibonacci работает неправильно"

AI: execute_function("src/math.rs", "fibonacci", args=[10])
Result: {result: [0, 1, 1, 2, 3, 5, 8, 13, 21, 35]}  # 35 неправильно!

AI: (анализирует код)
AI: "Обнаружена ошибка в вычислении последнего элемента"
AI: patch_file("src/math.rs", search="a + b", replace="a + b correctly")

AI: execute_function("src/math.rs", "fibonacci", args=[10])
Result: {result: [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]}  # правильно!

AI: "Баг исправлен!"
```

## Преимущества

### 1. AI становится инженером, а не генератором
AI может **сам проверять** свою работу, а не ждать пользователя.

### 2. Быстрая итерация
AI пишет → тестирует → фиксит → повторяет до success.

### 3. TDD-friendly
AI может писать тесты и реализацию итеративно.

### 4. Отладка
AI может выполнить функцию с разными аргументами для поиска бага.

## Сложность реализации
Высокая (7-10 дней)
- WASM sandbox для Rust: средняя (3 дня)
- Docker integration: средняя (3 дня)
- Test runner integration: низкая (2 дня)
- Resource limits + security: средняя (2 дня)

## Статус в gofer
❌ Отсутствует, но есть `run_diagnostics` (статический анализ)

## Зависимости
- WASM runtime (wasmer/wasmtime) или Docker
- Test frameworks (cargo test, pytest, jest, и т.д.)
- Resource limits (cgroups для Docker, WASI для WASM)

## Безопасность

### Обязательные ограничения
- ❌ Нет доступа к filesystem (кроме явно разрешённых)
- ❌ Нет доступа к network
- ❌ Нет выполнения системных команд
- ✅ Лимиты CPU/Memory
- ✅ Таймаут выполнения

### Подтверждение пользователя
```
AI: execute_function("untrusted.rs", "suspicious_function")

Prompt: "AI wants to execute code. Allow? (y/n)"
```

## Конфигурация

```toml
[execution]
# Включить execution sandbox
enabled = true

# Тип изоляции
isolation = "wasm"  # "wasm" | "docker" | "vm"

# Лимиты
max_memory_mb = 512
max_cpu_time_seconds = 5
default_timeout_seconds = 5

# Безопасность
allow_filesystem_access = false
allow_network_access = false
require_user_confirmation = true
```

## Альтернативы
- AI только генерирует код (не проверяет)
- Пользователь вручную тестирует (медленно)
- Статический анализ (не покрывает runtime ошибки)

## Связанные функции
- `run_diagnostics` — статический анализ
- `verify_patch` — проверка компиляции
- `benchmark` — замер производительности
