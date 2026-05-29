# gofer MCP - Roadmap для Production-Ready AI Code Assistant

> **Context:** Результаты глубокого тестирования и анализа использования gofer MCP в реальных сценариях работы над кодом.
> 
> **Goal:** Превратить gofer из "поисковика по коду" в полноценного AI-напарника программиста.

**Current Status:** ✅ 94% функционала работает отлично (33/33 тестов пройдены)
- 44 файла проиндексированы
- 597 code chunks
- 0 ошибок компиляции
- Semantic search, symbol analysis, Rust integration работают

---

## 🎯 Топ-8 Стратегических Направлений

### 1️⃣ Runtime Context - "Оживить" статический код

**Проблема:** gofer видит только статический код, но не понимает КАК он работает в реальности.

**Что нужно реализовать:**

```rust
// Новые MCP инструменты:
get_test_coverage(file: String) -> TestCoverageInfo
  // Возвращает: какие тесты покрывают этот файл/функцию
  // Процент покрытия по строкам
  // Непокрытые участки (gaps)

get_execution_traces(function: String, limit: usize) -> Vec<ExecutionTrace>
  // Типичные пути выполнения функции
  // Частые комбинации параметров
  // Call stacks из production/tests

get_runtime_examples(function: String) -> Vec<RuntimeExample>
  // Реальные примеры вызовов с данными
  // Input/output examples из тестов
  // Edge cases с реальными значениями

get_performance_hotspots(module: Option<String>) -> Vec<PerformanceHotspot>
  // Где код тормозит? (profiling data)
  // CPU/memory intensive участки
  // Рекомендации по оптимизации

find_error_patterns(file: String) -> Vec<ErrorPattern>
  // Где чаще всего падает?
  // Типичные exceptions/panics
  // Error recovery patterns
```

**Use Cases:**
- "Как оптимизировать embedder_stage?" → gofer показывает bottlenecks + типичные данные
- "Какие edge cases обрабатывает parse_file?" → gofer показывает реальные примеры из тестов
- "Где может упасть этот код?" → gofer показывает panic points + error handling gaps

**Implementation Plan:**
1. [ ] Интеграция с coverage tools (tarpaulin для Rust, nyc для TS)
2. [ ] Парсинг test execution results
3. [ ] Хранение runtime examples в SQLite
4. [ ] Сбор performance metrics (опционально, через instrumentation)

**Priority:** 🔥🔥🔥 **Critical** - превратит gofer из "читалки" в "понимателя поведения"

---

### 2️⃣ Code Evolution - Временное измерение

**Проблема:** gofer хранит один snapshot. Но код ЭВОЛЮЦИОНИРУЕТ, и история важна для понимания "почему так".

**Что нужно реализовать:**

```rust
get_code_evolution(file: String, symbol: Option<String>, months: usize) -> CodeEvolution
  // Как менялась функция/файл за N месяцев
  // Ключевые рефакторинги с commit messages
  // Визуализация: "было 50 строк → стало 200"

find_hotspots(file: String) -> Vec<CodeHotspot>
  // Какие строки часто меняют? (churn analysis)
  // Топ-10 самых нестабильных участков
  // Корреляция с багами

find_stable_core(module: String) -> Vec<StableCode>
  // Что никогда не трогают? (stable = battle-tested)
  // Core functionality vs experimental
  // Кандидаты для документирования

get_refactoring_history(symbol: String) -> Vec<Refactoring>
  // История рефакторингов символа
  // Причины изменений (из commit messages)
  // Breaking changes

find_all_todos() -> Vec<TodoItem>
  // TODO/FIXME/HACK по всему проекту
  // Группировка по модулям
  // Приоритизация по важности участка

get_code_churn(period: String, threshold: usize) -> Vec<ChurnMetrics>
  // Какие файлы нестабильны? (много изменений)
  // Индикатор проблемных областей
  // Рекомендации: "рассмотреть рефакторинг"
```

**Use Cases:**
- "Почему SqliteStorage такой сложный?" → История: был 50 строк, выросло до 1800 за 6 месяцев
- "Какие участки кода рискованно менять?" → Hotspots + связь с багами
- "Что в проекте требует внимания?" → TODO aggregation + churn analysis

**Implementation Plan:**
1. [ ] Расширить git integration: blame на уровне строк
2. [ ] Churn analysis через git log --numstat
3. [ ] TODO/FIXME parser с контекстом
4. [ ] Хранить historical snapshots в SQLite
5. [ ] Визуализация evolution (опционально, через markdown/ASCII charts)

**Priority:** 🔥🔥 **High** - добавит временное измерение, покажет "путь к текущему состоянию"

---

### 3️⃣ Human Context - Люди и решения

**Проблема:** Код пишут люди, принимают архитектурные решения. Этого контекста критически не хватает!

**Что нужно реализовать:**

```rust
get_code_owners(file: String) -> Vec<CodeOwner>
  // Кто эксперт в этом модуле? (по git history)
  // % вклада разных авторов
  // Контакты для вопросов

get_design_decisions(module: String) -> Vec<ArchitectureDecision>
  // Почему так спроектировано?
  // Парсинг ADR (Architecture Decision Records)
  // Ключевые решения из commit messages

get_related_discussions(file: String, line: Option<usize>) -> Vec<Discussion>
  // PRs, issues, code review comments об этом коде
  // Контекст изменений
  // Resolved/unresolved discussions

search_similar_problems(description: String) -> Vec<HistoricalIssue>
  // Похожие баги/фичи в истории проекта
  // Semantic search по issues
  // Решения которые сработали/не сработали

get_rejected_approaches(feature: String) -> Vec<RejectedApproach>
  // Что пробовали и отвергли?
  // Причины отказа
  // "Не делайте так, мы уже пробовали"
```

**Integrations Required:**
- GitHub Issues API (связать код ↔ issues)
- GitHub PRs API (найти discussions, reviews)
- GitHub Projects (текущие задачи)
- ADR parser (markdown документация)
- CODEOWNERS file parsing

**Use Cases:**
- "Почему используем fastembed, а не OpenAI?" → Находит Issue #15 с обсуждением
- "Кого спросить про indexer?" → Показывает @pa-khan (80% commits)
- "Пробовали ли мы async indexing?" → Находит PR #45 (rejected: complexity)

**Implementation Plan:**
1. [ ] GitHub API integration (issues, PRs, reviews)
2. [ ] ADR parser и хранение в SQLite
3. [ ] Code ownership analysis (git log + CODEOWNERS)
4. [ ] Semantic search по historical issues
5. [ ] Link code locations ↔ GitHub URLs

**Priority:** 🔥🔥 **High** - даст доступ к reasoning за решениями

---

### 4️⃣ Index Quality - Полнота и свежесть

**Проблема:** Обнаружены пустые results (trait impls, references, summaries). В production это неприемлемо!

**Что нужно реализовать:**

```rust
get_index_status() -> IndexStatus
  // Что проиндексировано, что нет?
  // % покрытия: symbols, references, embeddings, summaries
  // Last sync timestamp
  // Queue status

get_index_completeness(module: String) -> CompletenessReport
  // Детальный отчет по модулю
  // Missing: trait impls, macro expansions, etc.
  // Рекомендации: "reindex required"

force_reindex(path: String, priority: Priority) -> IndexingTask
  // Переиндексировать СЕЙЧАС (не ждать watcher)
  // Priority: High (blocking) | Low (background)
  // Progress tracking

validate_index() -> Vec<IndexingIssue>
  // Найти gaps и inconsistencies
  // Broken references
  // Outdated embeddings (model changed)
  // Corrupted data

get_indexing_queue() -> Vec<QueuedFile>
  // Что в очереди на индексацию?
  // ETA для каждого файла
  // Возможность изменить приоритет

estimate_index_time(path: String) -> Duration
  // Сколько займет индексация?
  // Учитывает: размер файла, язык, dependencies
```

**Improvements:**
1. **Smart prioritization**
   - Сначала часто используемые файлы (по git log)
   - Core modules vs experimental
   - User workspace (файлы которые я редактирую)

2. **Incremental updates**
   - Не переиндексировать всё при изменении одного файла
   - Инкрементальный update зависимых файлов
   - Invalidate только affected chunks

3. **Background indexing**
   - Индексировать пока пользователь работает
   - Low-priority queue для несрочных файлов
   - CPU throttling (не мешать работе)

4. **Health monitoring**
   - Алерты если индекс устарел (> 1 hour)
   - Автоматический repair при corruption
   - Metrics: indexing speed, queue length

**Use Cases:**
- Запуск gofer на новом проекте: прогресс-бар с ETA
- После git pull: "3 files need reindexing (ETA: 30s)"
- Пустой результат search: "Hint: module not fully indexed, run force_reindex?"

**Implementation Plan:**
1. [ ] Index health metrics в SQLite
2. [ ] Completeness checker (scan all files vs indexed)
3. [ ] Priority queue для indexing tasks
4. [ ] force_reindex tool
5. [ ] Background indexer с CPU limits
6. [ ] Incremental update strategy

**Priority:** 🔥🔥 **High** - без надежного индекса всё остальное не имеет смысла

---

### 5️⃣ Multi-Version Code Management - Версионирование внутри проекта

**Проблема:** В реальных проектах часто сосуществуют **несколько версий одного API** в разных папках (например, `api/v1/`, `api/v2/`, `api/v3/`). Текущая система subprojects работает **только с манифестами** (Cargo.toml, package.json), что не покрывает этот сценарий.

**Concrete Example:**
```
my-project/
├── Cargo.toml                    # ← Root manifest (defines one subproject)
├── api/
│   ├── v1/
│   │   ├── handlers.rs          # User handler V1 (legacy)
│   │   ├── models.rs            # Simple User model
│   │   └── auth.rs              # Basic auth
│   ├── v2/
│   │   ├── handlers.rs          # User handler V2 (current)
│   │   ├── models.rs            # Extended User model + validation
│   │   └── auth.rs              # JWT auth
│   └── v3/
│       ├── handlers.rs          # User handler V3 (beta)
│       ├── models.rs            # User model with RBAC
│       └── auth.rs              # OAuth2 auth
├── frontend/
│   ├── legacy/                  # Old Vue 2 app
│   │   └── components/
│   └── modern/                  # New Vue 3 app
│       └── components/
└── database/
    ├── migrations-v1/           # Schema for V1 API
    └── migrations-v2/           # Schema for V2 API
```

**Текущее поведение (проблемы):**
1. **Все файлы считаются одним проектом** - нет разделения между v1/v2/v3
2. **Поиск не различает версии:**
   - Query: `"user authentication handler"` 
   - Result: возвращает ВСЕ 3 версии `auth.rs` без фильтрации
   - User confused: "Какую версию использовать?"
3. **Нет контекста версии:**
   - gofer показывает `handlers.rs:45` - но из какой версии?
   - При навигации можно случайно попасть не в ту версию
4. **Duplicate symbols:**
   - Символ `UserHandler` существует в 3 экземплярах
   - `get_callers("UserHandler")` возвращает микс из всех версий
   - Невозможно фильтровать: "покажи callers только для V2"
5. **Migration confusion:**
   - "В какой версии появилась эта функция?"
   - "Какие изменения между V1 и V2 `User` модели?"
   - "Какой код еще использует legacy V1?"

**Real-World Impact:**
- **Microservices:** `services/auth-service-v1/`, `services/auth-service-v2/`
- **API versioning:** REST API v1/v2/v3 в одном репозитории
- **Frontend rewrites:** legacy React app + new Next.js app в одном монорепо
- **Database schemas:** старые и новые migrations
- **Protocol versions:** gRPC v1/v2, GraphQL schemas v1/v2
- **Library evolution:** `utils/old/`, `utils/new/` во время рефакторинга

**Что нужно реализовать:**

#### Solution 1: Structural Zones (автоматическая детекция)

```rust
// Новая MCP функция:
get_version_zones() -> Vec<VersionZone>
  // Автоматически детектит паттерны версионирования:
  // - api/v1, api/v2, api/v3
  // - services/auth-v1, services/auth-v2
  // - frontend/legacy, frontend/modern
  // - migrations-2023, migrations-2024
  // - components/old, components/new

search_in_zone(query: String, zone: String, limit: usize) -> Results
  // Поиск ТОЛЬКО в конкретной версии
  // Example: search_in_zone("auth handler", "api/v2", 10)
  
compare_versions(symbol: String, zone1: String, zone2: String) -> VersionDiff
  // Сравнить один символ между версиями
  // Example: "User model in v1 vs v2"
  // Returns: добавленные/удаленные поля, изменения типов

find_version_usages(zone: String) -> UsageReport
  // Кто использует старую версию?
  // Example: "Какой код еще ссылается на V1 API?"
  // Critical для планирования deprecation
  
migrate_path(from_zone: String, to_zone: String) -> MigrationGuide
  // Какие изменения нужны для миграции?
  // Breaking changes между версиями
  // Автоматические рекомендации
```

**Detection Heuristics:**
```rust
// Паттерны папок (ranked by confidence):
// High confidence:
- /v\d+/                    # /v1/, /v2/, /v10/
- /-v\d+/                   # /auth-v1/, /auth-v2/
- /version-\d+/             # /version-1/, /version-2/

// Medium confidence:
- /legacy/ vs /current/
- /old/ vs /new/
- /deprecated/ vs /active/
- /\d{4}/ (years)          # /2023/, /2024/ for migrations

// Low confidence (need user confirmation):
- /alpha/, /beta/, /stable/
- /experimental/, /production/
```

#### Solution 2: Explicit Configuration

```toml
# .gofer/config.toml
[version_zones]
"api/v1" = { label = "API v1 (legacy)", status = "deprecated", end_of_life = "2025-01-01" }
"api/v2" = { label = "API v2 (stable)", status = "current" }
"api/v3" = { label = "API v3 (beta)", status = "preview" }

"frontend/legacy" = { label = "Vue 2 App", status = "maintenance" }
"frontend/modern" = { label = "Vue 3 App", status = "active" }

[version_zones.rules]
# Автоматические правила для поиска
default_zone = "api/v2"              # Приоритет при неоднозначности
exclude_deprecated = true            # Не показывать deprecated по умолчанию
warn_on_legacy_usage = true          # Warning если используешь V1
```

#### Solution 3: Metadata Enrichment

**Extend SQLite schema:**
```sql
-- migrations/013_version_zones.sql
CREATE TABLE version_zones (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path_prefix TEXT NOT NULL UNIQUE,        -- "api/v1", "api/v2"
    label       TEXT NOT NULL,                -- "API v1 (legacy)"
    version     TEXT,                         -- "1.0", "2.0", "3.0-beta"
    status      TEXT NOT NULL,                -- "deprecated", "current", "preview"
    created_at  DATETIME,
    deprecated_at DATETIME,
    end_of_life DATETIME
);

-- Tag files with their zone
ALTER TABLE files ADD COLUMN version_zone_id INTEGER REFERENCES version_zones(id);

-- Tag symbols with their zone (for deduplication)
ALTER TABLE symbols ADD COLUMN version_zone_id INTEGER REFERENCES version_zones(id);

CREATE INDEX idx_files_zone ON files(version_zone_id);
CREATE INDEX idx_symbols_zone ON symbols(version_zone_id);
```

**Enhanced search queries:**
```rust
// При поиске символов - показывать версию:
"UserHandler (api/v1)" <- deprecated
"UserHandler (api/v2)" <- current ✓
"UserHandler (api/v3)" <- preview

// При get_callers - группировать по версиям:
Callers of "authenticate()":
  api/v2:
    - payments/handler.rs:123
    - orders/controller.rs:45
  api/v1 (deprecated):
    - legacy/cron.rs:67  <- WARNING: uses deprecated API
```

#### Solution 4: Cross-Version Analysis

```rust
get_version_adoption() -> VersionAdoption
  // Статистика использования версий:
  // - V1: 15% codebase (120 files) - LEGACY
  // - V2: 80% codebase (650 files) - CURRENT
  // - V3: 5% codebase (40 files) - PREVIEW
  
find_migration_candidates() -> Vec<MigrationCandidate>
  // Файлы которые можно мигрировать с V1 на V2
  // Ранжированные по сложности
  
detect_mixed_version_usage(file: String) -> Vec<VersionConflict>
  // Найти файлы которые импортируют из разных версий
  // Example: импортирует v1.User + v2.Auth (code smell!)
  
version_timeline() -> Timeline
  // История версий:
  // - V1: 2022-01 .. 2024-06 (deprecated)
  // - V2: 2023-06 .. active
  // - V3: 2024-12 .. beta
```

**Use Cases:**
1. **Safe search**: 
   - User: "find authentication handler"
   - gofer: "Found in 3 zones: v2 (current, recommended), v1 (deprecated), v3 (preview). Show v2?"

2. **Migration planning**:
   - User: "What still uses V1 API?"
   - gofer: "15 files still reference v1/, here's migration guide for each"

3. **Code review**:
   - gofer: "⚠️ Warning: this PR imports from api/v1 (deprecated since 2024-06)"

4. **Onboarding**:
   - New dev: "Which version should I use?"
   - gofer: "Use api/v2 (current), v1 is deprecated, v3 is experimental"

5. **Deprecation audit**:
   - User: "Can we remove V1?"
   - gofer: "No - 12 files still depend on it. Here's what needs migration."

**Implementation Plan:**
1. [ ] **Phase 1: Detection** (2 weeks)
   - Implement heuristics для автоматической детекции версий
   - Add `version_zones` SQLite table
   - Scan project и предложить zones

2. [ ] **Phase 2: Configuration** (1 week)
   - TOML config для explicit zones
   - UI для review/confirm detected zones
   - Store в database

3. [ ] **Phase 3: Search Integration** (2 weeks)
   - Extend search с zone filtering
   - Show version tags в результатах
   - Default zone preferences

4. [ ] **Phase 4: Analysis Tools** (2 weeks)
   - `compare_versions()` - diff между версиями
   - `find_version_usages()` - usage tracking
   - `detect_mixed_version_usage()` - conflicts

5. [ ] **Phase 5: Migration Helpers** (2 weeks)
   - Migration path suggestions
   - Breaking changes detection
   - Deprecation warnings

**Priority:** 🔥🔥 **High** - критично для реальных enterprise проектов с API versioning

**Dependencies:**
- Requires: subprojects infrastructure (already exists ✅)
- Extends: search, symbol resolution, reference tracking
- Enables: better deprecation management, migration planning

---

### 6️⃣ Smart Ranking - Релевантность для больших проектов

**Проблема:** В больших проектах (1000+ файлов) search возвращает слишком много результатов.

**Что нужно реализовать:**

```rust
search_ranked(
    query: String,
    context: RankingContext {
        recent_changes: bool,      // Приоритет недавно измененному
        test_coverage: bool,       // Приоритет покрытому тестами
        code_churn: ChurnFilter,   // low/medium/high - избегать нестабильный
        ownership: OwnershipFilter, // core_team/all
        my_workspace: bool,        // Приоритет моим файлам
        stability: StabilityFilter, // stable/all
    }
) -> RankedResults
```

**Ranking Factors (configurable weights):**
1. **Semantic similarity** (baseline) - 40%
   - Current embedding-based search
   
2. **Recency** - 20%
   - Когда последний раз менялся файл
   - Recent = more relevant (отражает current architecture)
   
3. **Stability** - 15%
   - Как часто меняется (churn analysis)
   - Stable code = important, battle-tested
   - High churn = experimental or problematic
   
4. **Test coverage** - 10%
   - % покрытия тестами
   - Tested code = reliable, documented behavior
   
5. **Code ownership** - 10%
   - Core team > external contributors
   - Main author > occasional contributor
   
6. **Personal relevance** - 5%
   - Моя история работы с файлом
   - Недавно открытые/редактированные
   - Bookmarked files

**Additional Filters:**
- Exclude: deprecated code, archived modules
- Language-specific: prefer idiomatic code
- Domain-specific: backend vs frontend preference

**Use Cases:**
- "authentication implementation" → TOP-5 ranked вместо 100 результатов
- Фильтр "stable only" → избежать experimental code
- "Show me what core team wrote" → ownership filter

**Implementation Plan:**
1. [ ] Ranking engine с configurable weights
2. [ ] Collect ranking signals: recency, churn, ownership
3. [ ] Personal workspace tracking (what I work with)
4. [ ] A/B testing framework для weights
5. [ ] UI для настройки preferences

**Priority:** 🔥 **Medium** - критично для масштабирования на большие проекты

---

### 7️⃣ Language Deep Dive - Специализация

**Проблема:** Каждый язык уникален, generic tools недостаточно для deep understanding.

**Rust-специфичное:**

```rust
explain_lifetime(code: String) -> LifetimeExplanation
  // Объяснить lifetime аннотации human-readable
  // Визуализация lifetime scope
  // Частые ошибки и как их избежать

find_all_unsafe() -> Vec<UnsafeBlock>
  // Все unsafe блоки с причинами (из комментариев)
  // Safety invariants
  // Audit status

check_send_sync(type_name: String) -> ThreadSafetyReport
  // Проверить thread-safety
  // Почему NOT Send/Sync (если не реализовано)
  // Рекомендации

explain_macro_expansion(macro_call: String) -> MacroExpansion
  // Пошаговое раскрытие макроса
  // Intermediate steps
  // Final expanded code

find_panic_points(file: String) -> Vec<PanicPoint>
  // Где может panic? (unwrap, expect, panic!, assert!)
  // Рекомендации: Result<T> или Option<T>

suggest_error_handling(function: String) -> ErrorHandlingSuggestions
  // Где нужен Result<T> вместо unwrap
  // Где добавить error context (anyhow)
```

**TypeScript-специфичное:**

```typescript
infer_missing_types(file: String) -> TypeInference
  // Вывести типы для неаннотированного кода
  // Предложить type annotations

find_any_types() -> Vec<AnyUsage>
  // Найти все `any` (code smell)
  // Предложить правильные типы

suggest_interface(object: String) -> InterfaceDefinition
  // Предложить interface для объекта
  // Extract common shape

check_null_safety(function: String) -> NullSafetyReport
  // Где может быть null/undefined?
  // Рекомендации: optional chaining, nullish coalescing
```

**Python-специфичное:**

```python
trace_dynamic_imports(module: String) -> ImportTrace
  // Где определен этот класс? (dynamic imports)
  // Resolve import chains

find_missing_type_hints() -> Vec<UntypedFunction>
  // Что без type hints?
  // Автоматическая генерация hints (где возможно)

check_duck_typing(function: String) -> DuckTypingReport
  // Какие протоколы ожидаются?
  // Structural typing analysis
```

**Use Cases:**
- Rust: "Почему этот тип не Send?" → Детальное объяснение
- TypeScript: "Добавь типы в этот файл" → Auto-inference + suggestions
- Python: "Где определен MyClass?" → Trace через dynamic imports

**Implementation Plan:**
1. [ ] Rust: unsafe analyzer, lifetime explainer
2. [ ] TypeScript: type inference, any detector
3. [ ] Python: import tracer, type hint generator
4. [ ] Language-specific linters integration
5. [ ] Educational content (explain как правильно)

**Priority:** 🔥 **Medium** - повысит ценность для специализированной работы

---

### 8️⃣ Interactive Learning - Обратная связь и персонализация

**Проблема:** gofer сейчас read-only. Нужен диалог и адаптация под пользователя!

**Personal Workspace:**

```rust
save_workspace(name: String, files: Vec<String>) -> Workspace
  // Сохранить набор файлов для текущей задачи
  // Например: "feature-auth", "bug-indexer"

load_workspace(name: String) -> Vec<String>
  // Вернуться к задаче
  // Восстановить контекст работы

get_my_hotspots() -> Vec<FileHotspot>
  // Файлы которые я часто читаю/редактирую
  // Персональные frequently accessed

save_search_shortcut(name: String, query: String, filters: Filters)
  // Сохранить частый запрос
  // Например: "my-auth-code" → search auth + my files only

get_recent_explorations() -> Vec<ExplorationHistory>
  // История моих исследований кода
  // "Вчера я разбирался с X, сегодня продолжаю"
```

**Code Annotations:**

```rust
annotate_code(file: String, line: usize, note: String, type: AnnotationType)
  // Добавить мою заметку к коду
  // Types: NOTE, WARNING, TODO, QUESTION, LINK

mark_as_legacy(file: String, reason: String)
  // Пометить "это старый код, не трогать"

mark_pattern(file: String, pattern_type: String)
  // "это правильный паттерн - делать так"
  // "это anti-pattern - избегать"

link_to_issue(file: String, line: usize, issue_url: String)
  // Связать код с issue/PR
  // Автоматически создавать links

create_bookmark(file: String, line: usize, description: String)
  // Закладка в коде
  // Быстрый возврат к важным местам
```

**Guided Exploration:**

```rust
explain_flow(from: Location, to: Location) -> CodeFlow
  // Проведи от A до B (например: HTTP request → database)
  // Step-by-step trace с объяснениями
  // Interactive walkthrough

create_tutorial(topic: String) -> Tutorial
  // Создать туториал "как работает X"
  // Автоматически из кода + комментариев
  // Markdown или interactive

find_learning_path(goal: String) -> Vec<LearningStep>
  // Какие файлы читать чтобы понять X?
  // Упорядоченный список с обоснованием
  // "Начни с A, потом B, затем C"

ask_question(question: String, context: Vec<String>) -> Answer
  // Задать вопрос о коде в контексте
  // gofer отвечает используя indexed knowledge
  // С ссылками на код
```

**Learning from Usage:**

```rust
track_my_patterns() -> UsagePatterns
  // Какие файлы я часто читаю вместе?
  // Какие search queries я повторяю?
  // Мой стиль работы

suggest_next_file(current_file: String) -> Vec<String>
  // "Обычно после A ты смотришь B"
  // Predictive navigation

auto_create_shortcuts()
  // Автоматически создавать shortcuts из patterns
  // "Ты искал 'auth' 10 раз, создать shortcut?"
```

**Use Cases:**
- Начало работы: "load_workspace('feature-payments')" → gofer восстанавливает контекст
- Код review: добавляю аннотации "проверить thread-safety здесь"
- Onboarding: "create_tutorial('how embeddings work')" → интерактивный туториал
- Daily work: gofer предлагает "обычно ты смотришь tests после изменения impl"

**Implementation Plan:**
1. [ ] Workspace management (SQLite table)
2. [ ] Annotations system (overlay поверх code)
3. [ ] Usage tracking (privacy-aware)
4. [ ] Tutorial generator
5. [ ] Flow tracer (call graph + data flow)
6. [ ] Q&A system (RAG over indexed code)

**Priority:** 🔥 **Medium** - превратит gofer в персонального AI-напарника

---

## 📊 Приоритизация Implementation

### Phase 1: Foundation (Critical) - 2-3 месяца
- ✅ Runtime Context (tests, examples, coverage)
- ✅ Index Quality (health, validation, force reindex)
- ✅ Code Evolution (history, churn, hotspots)

**Goal:** Надежная база + понимание behavior

### Phase 2: Intelligence (High Priority) - 2-3 месяца  
- ✅ Human Context (GitHub integration, ADR, ownership)
- ✅ Smart Ranking (multi-factor, personalization)

**Goal:** Релевантность + контекст решений

### Phase 3: Specialization (Medium Priority) - 2-3 месяца
- ✅ Language Deep Dive (Rust, TS, Python specifics)
- ✅ Interactive Learning (workspace, annotations, tutorials)

**Goal:** Экспертиза + адаптация под пользователя

---

## 🎯 Success Metrics

### Quantitative:
- Index completeness: **> 95%** coverage
- Search precision: **> 80%** relevant in top-5 results
- Response time: **< 500ms** для поиска, **< 2s** для сложного анализа
- Uptime: **> 99%** (индекс всегда актуален)

### Qualitative:
- "Могу найти что угодно за < 1 минуту"
- "Понимаю why, а не только what"
- "gofer знает больше о проекте, чем любой новый разработчик"
- "gofer адаптировался под мой workflow"

---

## 💡 Long-term Vision

gofer как **AI Senior Developer** в команде:
- Знает всю историю проекта (evolution)
- Понимает архитектурные решения (human context)
- Видит как код работает (runtime context)
- Учится от каждого разработчика (personalization)
- Помогает onboarding новых людей (tutorials)
- Предупреждает о проблемах (proactive alerts)

**Not just a tool, but a team member** 🚀

---

## 📝 Notes

**Date:** 2026-02-16  
**Authors:** @pa-khan (architect), Claude (implementation & analysis)  
**Status:** RFC - Request for Comments  
**Next Steps:** 
1. Review priorities
2. Create detailed specs for Phase 1
3. Start with Runtime Context + Index Quality
4. Iterate based on usage feedback

**Feedback Welcome!** Open issues or PRs to discuss priorities, add use cases, or propose new features.
