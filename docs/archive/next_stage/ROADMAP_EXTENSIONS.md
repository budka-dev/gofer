# gofer MCP - Extended Roadmap (Community Insights)

> **Context:** Дополнительные идеи и возможности, выявленные в процессе глубокого тестирования и реального использования gofer MCP.
> 
> **Source:** Анализ пользовательского опыта, токен-эффективности, real-time сценариев работы.
> 
> **Status:** RFC - Дополнение к основному ROADMAP.md

**Date:** 2026-02-16  
**Contributors:** Claude (AI analysis based on real usage)

---

## 🎯 Новые направления, основанные на реальном опыте

### 1️⃣ **Token-Efficient Context (Токен-экономное чтение)**

**Проблема:**
При работе с LLM контекст ограничен, и чтение больших файлов расходует токены неэффективно. Например:
- Файл 20KB = ~5000 токенов
- 80% контента — тела функций, которые часто не нужны
- При чтении 10 файлов → 50K токенов, половина — waste

**Что предлагается:**

```rust
read_file_smart(
    file: String,
    mode: ReadMode,
    focus_symbols: Vec<String>,
) -> SmartFileContent

enum ReadMode {
    SkeletonOnly,           // Только сигнатуры функций/типов
    SignaturesAndDocs,      // + docstrings и комментарии
    WithKeyFunctions,       // + реализация важных функций (по usage stats)
    Full,                   // Полный файл (как сейчас)
}

// Дополнительные инструменты
read_function_only(file: String, function: String) -> FunctionContent
  // Читать ТОЛЬКО одну функцию с контекстом (импорты, типы)

read_types_only(file: String) -> Vec<TypeDefinition>
  // Только определения типов (struct, enum, interface)

read_dependencies_minimal(file: String, depth: usize) -> MinimalContext
  // Импорты + минимальные определения из зависимостей
```

**Примеры использования:**

```rust
// Сейчас:
Read("src/indexer/embedder.rs") 
// → 6000 токенов (весь файл)

// С новой фичей:
read_file_smart("src/indexer/embedder.rs", SkeletonOnly, [])
// → 1200 токенов (только сигнатуры)

read_file_smart("src/indexer/embedder.rs", SignaturesAndDocs, ["EmbedderPool::embed"])
// → 2000 токенов (сигнатуры + полная реализация embed() + её документация)
```

**Преимущества:**
- **3-5× экономия токенов** для большинства сценариев
- Быстрее анализ (меньше данных для LLM)
- Можно читать больше файлов в пределах context window
- Дешевле (токены = деньги)

**Техническая реализация:**
- Skeleton функция уже есть в `src/indexer/parser/skeleton.rs`
- Нужно только обернуть в MCP tool
- Добавить фильтрацию по символам

**Priority:** 🔥🔥🔥 **Critical**  
**Effort:** Low (1-2 дня)  
**Impact:** Огромный - сразу улучшает все сценарии использования

---

### 2️⃣ **Real-time Change Impact Analysis (Живой diff анализ)**

**Проблема:**
gofer видит uncommitted changes через `git_diff`, но не анализирует их **impact** на остальной код. Разработчик узнаёт о проблемах только после компиляции/тестов.

**Что предлагается:**

```rust
analyze_uncommitted_changes() -> ChangeImpact {
    modified_symbols: Vec<Symbol>,           // Что изменено
    affected_callers: Vec<CallerLocation>,   // Кто вызывает изменённое
    broken_references: Vec<BrokenRef>,       // Что может сломаться
    test_coverage_delta: TestCoverageDiff,   // Новые участки без тестов
    similar_past_changes: Vec<HistoricalChange>, // История похожих изменений
    risk_level: RiskLevel,                   // Low/Medium/High
}

suggest_tests_for_changes() -> Vec<TestSuggestion>
  // Какие тесты запустить на основе changed code
  // Приоритет: affected functions + historical failures

preview_build_impact() -> BuildImpact
  // Оценка: сколько файлов перекомпилируется (Rust)
  // Типичное время компиляции для таких изменений
  // Dependency graph analysis

check_breaking_changes() -> Vec<BreakingChange>
  // Public API изменения
  // Signature changes в exported functions
  // Кто из внешних модулей пострадает

suggest_migration_path() -> MigrationGuide
  // Как обновить callers после breaking change
  // Code snippets для миграции
```

**Use Cases:**

```
Сценарий 1: Изменение сигнатуры
User: *меняет fn embed(texts: Vec<String>) на fn embed(texts: &[String])*
gofer: "⚠️ 7 callers affected:
  - pipeline.rs:587 (critical path - HIGH priority)
  - service.rs:42 (moderate usage)
  Suggested fix: change .to_vec() to &texts
  Похожее изменение 2 недели назад потребовало обновить 3 теста"

Сценарий 2: Добавление нового кода
User: *добавляет новую функцию parse_python()*
gofer: "✅ No breaking changes
  ⚠️ Function не покрыта тестами
  💡 Похожая функция parse_rust() имеет 5 тестов, рекомендую аналогичные"

Сценарий 3: Рефакторинг
User: *переименовывает SqliteStorage → DatabaseStorage*
gofer: "🔍 45 references found across 12 files
  Estimated build time: ~30s (12 files to recompile)
  ✅ All usages are internal (no public API impact)"
```

**Преимущества:**
- **Проактивная помощь** во время разработки
- Знаешь impact ДО компиляции
- Избегаешь "сломал 10 файлов и не заметил"
- gofer становится **co-pilot**, а не просто reference tool

**Техническая реализация:**
1. Интеграция с `git_diff` (уже есть)
2. Cross-reference analysis через `get_callers()` (уже есть)
3. Test coverage tracking (новое)
4. Historical change analysis (из git log)
5. Real-time monitoring uncommitted files

**Priority:** 🔥🔥🔥 **Critical**  
**Effort:** Medium (1-2 недели)  
**Impact:** Превращает gofer в real-time development assistant

---

### 3️⃣ **Semantic Diff Between Versions (Семантическая разница)**

**Проблема:**
`get_code_evolution` из ROADMAP показывает **что** менялось (строки, файлы), но не объясняет **семантическую разницу** - как изменилось поведение.

**Что предлагается:**

```rust
explain_diff(
    from_commit: String,
    to_commit: String,
    file: String,
) -> SemanticDiff {
    // Высокоуровневые изменения
    added_capabilities: Vec<String>,      // "Добавлена поддержка GPU"
    removed_capabilities: Vec<String>,    // "Убрана синхронная API"
    behavioral_changes: Vec<String>,      // "Теперь кэширует результаты"
    breaking_changes: Vec<BreakingChange>, // С деталями affected code
    
    // Performance impact
    performance_impact: Option<PerformanceImpact>,
    // "Ожидается 2× ускорение из-за batching"
    
    // AI-generated summary
    plain_english_summary: String,
    // "Рефакторинг пула эмбеддингов для поддержки динамического масштабирования.
    //  Главное изменение: Arc вместо Option для владения инстансами.
    //  Breaking change: метод embed() теперь async."
    
    // Code-level details
    modified_functions: Vec<FunctionDiff>,
    new_dependencies: Vec<Dependency>,
    removed_dependencies: Vec<Dependency>,
}

compare_implementations(
    symbol: String,
    version_a: String,  // commit/tag/branch
    version_b: String,
) -> ImplementationDiff {
    side_by_side_code: (String, String),
    key_differences: Vec<KeyDifference>,
    complexity_change: ComplexityMetrics,
    // Cyclomatic complexity, LOC, nesting depth
    
    algorithmic_differences: Vec<AlgorithmChange>,
    // "Version A uses HashMap, Version B uses BTreeMap (sorted keys)"
}

find_regressions(from: String, to: String) -> Vec<Regression>
  // Автоматический поиск регрессий
  // Performance regressions (если есть benchmark data)
  // Functionality regressions (removed features)
  // Test coverage regressions

explain_why_changed(file: String, line: usize) -> ChangeReasoning
  // Почему эта строка менялась?
  // Парсинг commit messages + related issues
  // Timeline: когда и почему менялось
```

**Use Cases:**

```
Сценарий 1: Отладка регрессии
User: "Почему embedder стал медленнее после коммита abc123?"
gofer: *compare_implementations("EmbedderPool::embed", "abc123^", "abc123")*
→ "Добавлена проверка кэша (blake3 hashing, 15 строк)
   Performance impact: 
   - Cold start: +20ms overhead (hash computation)
   - Warm cache: 3× ускорение (skip re-embedding)
   Trade-off: slower first run, faster subsequent"

Сценарий 2: Code review
User: "Что изменилось в PR #42?"
gofer: *explain_diff("main", "feature-branch", "src/indexer/")*
→ "Added capabilities: GPU support через ort/cuda feature
   Breaking changes: 
   - EmbedderPool::new() теперь требует config parameter
   - Removed sync embed() method
   Migration: use embed().await вместо embed()"

Сценарий 3: Понимание истории
User: "Как SqliteStorage эволюционировал?"
gofer: *explain_diff("v0.1.0", "v0.5.0", "src/storage/sqlite.rs")*
→ "50 строк → 1800 строк (36× рост)
   Major additions:
   - Chunk caching (migration 010, commit xyz)
   - FTS5 search (migration 001)
   - Cross-reference resolution
   Performance: query speed 10× faster (added indexes)"
```

**Преимущества:**
- **Понимание эволюции** кода, не только diff
- Быстрый анализ PR/MR
- Debugging: "когда это сломалось?"
- Обучение: "как это работало раньше?"

**Техническая реализация:**
1. Git integration: checkout разных версий
2. AST comparison (tree-sitter на обе версии)
3. Semantic analysis (что изменилось в поведении)
4. LLM для plain english summary
5. Benchmark data integration (опционально)

**Priority:** 🔥🔥 **High**  
**Effort:** Medium (2-3 недели)  
**Impact:** Добавляет временное измерение к пониманию кода

---

### 4️⃣ **Multi-Repo Context (Кросс-проектный поиск)**

**Проблема:**
gofer работает в пределах одного проекта. В реальности команды работают на нескольких связанных проектах, и нужен кросс-проектный контекст.

**Что предлагается:**

```rust
search_across_projects(
    query: String,
    projects: Vec<String>,  // ["gofer", "frontend-app", "backend-api"]
    filters: CrossProjectFilters,
) -> MultiRepoResults {
    results_by_project: HashMap<String, Vec<SearchHit>>,
    cross_references: Vec<CrossProjectReference>,
    shared_patterns: Vec<SharedPattern>,
}

find_similar_code_in_other_projects(
    code_snippet: String,
    exclude_project: String,
) -> Vec<SimilarCodeMatch> {
    project: String,
    file: String,
    similarity_score: f32,
    can_reuse: bool,
    differences: Vec<String>,
}
  // "В проекте backend-api уже реализовано похожее"
  // Помогает избежать дублирования кода

get_shared_dependencies(projects: Vec<String>) -> DependencyMap
  // Какие библиотеки общие между проектами?
  // Конфликты версий (project A: tokio@1.35, project B: tokio@1.40)
  // Recommendations для унификации

find_api_consumers(api_project: String) -> Vec<Consumer>
  // Кто использует наш API из других проектов?
  // Impact analysis для breaking changes
  // Example: "3 проекта зависят от этого endpoint"

detect_code_duplication_across_repos() -> Vec<Duplication>
  // Найти дублированный код между проектами
  // Кандидаты для extraction в shared library

unified_search(query: String) -> UnifiedResults
  // Один поиск по всем проектам команды
  // Ранжирование учитывает cross-project relevance
```

**Use Cases:**

```
Сценарий 1: Переиспользование кода
User: "Как реализовать JWT аутентификацию для gofer?"
gofer: *search_across_projects("JWT authentication", ["backend-api", "auth-service"])*
→ "В backend-api/src/auth/jwt.rs найдена полная реализация:
   - JWT token generation
   - Middleware для проверки
   - Refresh token logic
   Можно переиспользовать (MIT license)"

Сценарий 2: Breaking changes impact
User: "Если я изменю API endpoint /api/search, что сломается?"
gofer: *find_api_consumers("gofer")*
→ "⚠️ 2 проекта используют этот endpoint:
   - frontend-app: src/services/search.ts (4 calls)
   - cli-tool: src/commands/query.rs (1 call)
   Рекомендую: версионирование API (/v2/search)"

Сценарий 3: Dependency management
User: "Можно ли обновить tokio до 1.40?"
gofer: *get_shared_dependencies(["gofer", "backend-api", "worker"])*
→ "Current versions:
   - gofer: tokio@1.35
   - backend-api: tokio@1.40 ✅
   - worker: tokio@1.30 ⚠️
   
   Recommendation: сначала обновить worker, потом gofer
   Breaking changes: tokio 1.30→1.35 minimal, 1.35→1.40 none"
```

**Преимущества:**
- **Избежание дублирования** работы
- **Cross-project consistency**
- **Impact analysis** для breaking changes
- **Shared knowledge** между проектами

**Техническая реализация:**
1. Multi-project registry (gofer может индексировать несколько проектов)
2. Cross-reference tracking (imports между проектами)
3. Unified search index
4. Dependency graph analyzer
5. Code similarity detection (embedding-based)

**Priority:** 🔥 **Medium** (зависит от team structure)  
**Effort:** High (3-4 недели)  
**Impact:** Критично для команд с microservices/multi-repo setup

---

### 5️⃣ **Embedding-Powered Code Review (Умный ревью)**

**Проблема:**
Code review часто mechanical и repetitive: "есть ли тесты?", "правильный ли стиль?", "нет ли дублирования?". gofer может автоматизировать рутинные проверки.

**Что предлагается:**

```rust
review_uncommitted_changes() -> CodeReviewReport {
    // Автоматические проверки
    style_issues: Vec<StyleIssue>,          // Нарушения style guide
    missing_tests: Vec<UntestedCode>,       // Что не покрыто тестами
    security_concerns: Vec<SecurityIssue>,  // Potential vulnerabilities
    performance_concerns: Vec<PerfIssue>,   // Неоптимальные паттерны
    
    // Semantic review (embedding-based)
    inconsistencies: Vec<Inconsistency>,
    // "В функции A используется Result<T>, а в похожей функции B - unwrap()"
    
    similar_code_exists: Vec<Duplication>,
    // "Похожая логика в module X, можно унифицировать"
    
    anti_patterns: Vec<AntiPattern>,
    // "Recursive lock acquisition (deadlock risk)"
    // "Unbounded Vec growth (memory leak risk)"
    
    better_patterns: Vec<PatternSuggestion>,
    // "В этом проекте обычно error handling делают через anyhow"
    // Learnt from golden_samples
    
    // Complexity metrics
    complexity_score: ComplexityScore,
    // Cyclomatic complexity, nesting depth, function length
    
    recommendations: Vec<ReviewComment>,
    // Готовые комментарии для PR review
}

suggest_improvements(file: String, function: String) -> Vec<Improvement>
  // Рефакторинг suggestions
  // "Эта функция 150 строк, рекомендуем разбить"
  // "Можно упростить через iterator chains"
  // "Duplicated code в строках 42-58 и 103-119"

check_against_project_patterns() -> Vec<PatternViolation>
  // Сравнить с golden_samples
  // "Error handling не соответствует project standard"
  // "Naming convention violation: use snake_case"

security_audit(scope: AuditScope) -> SecurityReport
  // SQL injection risks
  // XSS vulnerabilities
  // Unsafe Rust blocks without justification
  // Credential leaks (hardcoded tokens)
  // OWASP Top 10 checks

estimate_review_time() -> Duration
  // Сколько времени займёт review?
  // На основе сложности изменений
```

**Use Cases:**

```
Сценарий 1: Pre-commit автопроверка
User: *делает git add .*
gofer (автоматически): "
  ✅ Стиль: OK (rustfmt passed)
  ✅ Тесты: 3 новых теста добавлены
  ⚠️ Complexity: embedder_stage() 150 строк (рекомендуем <100)
  ⚠️ Performance: O(n²) loop в строке 87, можно оптимизировать
  💡 Похожий кэш-паттерн есть в storage.rs:42, извлечь в helper?
  
  Estimated review time: 15-20 минут (medium complexity)
"

Сценарий 2: PR review assistance
Reviewer: "gofer, что подозрительного в этом PR?"
gofer: *review_uncommitted_changes()*
→ "🔴 Security concern: 
     Line 156: SQL query построен через format!() - SQL injection risk
     Recommendation: использовать параметризованные queries (sqlx)
   
   🟡 Performance concern:
     Lines 203-215: Sync file I/O в async context - блокирует tokio runtime
     Recommendation: использовать tokio::fs
   
   🟢 Code quality: good, соответствует project standards"

Сценарий 3: Learning от хорошего кода
User: "Как правильно писать error handling в этом проекте?"
gofer: *check_against_project_patterns()*
→ "Project standard (из golden_samples):
   - Use anyhow::Result<T> для application errors
   - Use thiserror для library errors
   - Избегать unwrap() в production code
   - Добавлять context: .context('Meaningful message')?
   
   Examples: см. src/storage/sqlite.rs (golden sample)"
```

**Преимущества:**
- **Автоматизация** рутинных проверок
- **Consistency** в проекте
- **Security** - находит типичные уязвимости
- **Learning** - новые участники видят "как правильно"
- **Экономия времени** reviewers

**Техническая реализация:**
1. Integration с linters (rustfmt, clippy, eslint)
2. Test coverage analysis
3. Security scanner (regex patterns + semantic analysis)
4. Complexity metrics (cyclomatic, cognitive)
5. Pattern matching против golden_samples
6. Embedding similarity для finding duplications

**Priority:** 🔥🔥 **High**  
**Effort:** Medium-High (2-3 недели)  
**Impact:** Улучшает качество кода, экономит время review

---

### 6️⃣ **Performance Profiling Integration (Реальные метрики)**

**Проблема:**
В коде есть оптимизации (кэширование, batching, пулы), но gofer не знает **фактическую** производительность. Анализ основан только на статическом коде.

**Что предлагается:**

```rust
get_performance_profile(function: String) -> PerformanceProfile {
    // Runtime metrics
    avg_execution_time: Duration,
    percentiles: (Duration, Duration, Duration),  // p50, p95, p99
    memory_usage: MemoryStats,
    allocation_rate: AllocationStats,
    
    // Profiling data
    cpu_profile: Vec<HotSpot>,
    // Где тратится CPU внутри функции
    
    flame_graph: FlameGraphData,
    // Для визуализации call stack
    
    // Comparative analysis
    compared_to_similar: String,
    // "На 30% медленнее чем parse_rust()"
    
    bottlenecks: Vec<Bottleneck>,
    // Где конкретно тормозит
    
    optimization_suggestions: Vec<OptimizationHint>,
    // На основе профиля
}

find_slow_code(threshold_ms: u64, module: Option<String>) -> Vec<SlowFunction>
  // Все функции медленнее X ms
  // С контекстом: почему медленно, где оптимизировать
  // Ранжировано по impact (freq × duration)

track_performance_regression(from: String, to: String) -> Vec<Regression>
  // Что замедлилось между коммитами?
  // Автоматический benchmark comparison
  // Highlight подозрительных изменений

get_memory_hotspots() -> Vec<MemoryHotspot>
  // Где больше всего аллокаций?
  // Memory leaks detection
  // Heap profile analysis

analyze_cache_efficiency(cache_name: String) -> CacheStats
  // Hit rate, miss rate
  // Optimal cache size estimation
  // Eviction policy effectiveness

benchmark_comparison(
    baseline: String,
    current: String,
    suite: String,
) -> BenchmarkDiff
  // Сравнение benchmark results
  // Regression/improvement detection
  // Statistical significance
```

**Use Cases:**

```
Сценарий 1: Оптимизация
User: "Почему индексация медленная?"
gofer: *get_performance_profile("embedder_stage")*
→ "Function profile:
   Avg execution: 1.2s per batch
   
   Time breakdown:
   - Embedding (fastembed): 980ms (82%)
   - Cache lookup (SQLite):  150ms (12%)
   - Hashing (blake3):        50ms (4%)
   - Other:                   20ms (2%)
   
   Bottleneck: Embedding computation
   
   Optimization suggestions:
   1. Increase batch_size: 32 → 64
      Expected: +40% throughput (benchmark data)
   2. Use GPU if available (feature flag 'gpu')
      Expected: 3-5× speedup
   3. Cache hit rate: 35% - consider warming cache"

Сценарий 2: Regression detection
CI/CD: *track_performance_regression("main", "feature-branch")*
gofer: "⚠️ Performance regression detected:
   - parse_file(): 45ms → 67ms (+48% slower)
   - Cause: Added validation logic (lines 234-256)
   - Impact: Medium (called 100× per indexing)
   - Total overhead: +2.2s per full index
   
   Recommendation: Move validation to separate pass or make optional"

Сценарий 3: Memory optimization
User: "gofer использует слишком много памяти"
gofer: *get_memory_hotspots()*
→ "Memory analysis:
   Top allocators:
   1. EmbedderPool instances: 450MB (model weights)
   2. Chunk cache: 150MB (100k entries)
   3. LanceDB mmap: 120MB
   
   Recommendations:
   - scale_down embedder pool when idle (already implemented ✅)
   - Evict chunk cache: limit 100k entries (already configured ✅)
   - Consider smaller embedding model (384d → 256d): -35% memory"
```

**Преимущества:**
- **Data-driven** оптимизация, не гадание
- **Regression detection** в CI/CD
- **Production insights** (если интегрировано)
- **Guided optimization** с конкретными советами

**Техническая реализация:**
1. Integration с benchmark tools:
   - `cargo bench` results → SQLite
   - CI benchmark runs → historical data
2. Profiling integration (опционально):
   - `perf` / `flamegraph` data
   - Memory profiler (jemalloc stats)
3. Production metrics (если включено):
   - Tracing spans → performance database
   - Aggregate statistics
4. Analysis engine:
   - Statistical comparison (t-test для regressions)
   - Bottleneck identification
   - Optimization suggestions (rule-based + ML)

**Priority:** 🔥🔥 **Medium-High**  
**Effort:** High (3-4 недели)  
**Impact:** Превращает gofer в performance advisor

---

### 7️⃣ **Natural Language Query Interface (Вопросы на естественном языке)**

**Проблема:**
MCP tools — структурированные вызовы, требуют знания API. Но часто вопросы нечеткие: "покажи где тормозит", "что может сломаться", "как это работает".

**Что предлагается:**

```rust
ask(question: String, context: AskContext) -> NaturalAnswer {
    // gofer переводит NL вопрос в серию MCP tool calls
    // Агрегирует результаты
    // Отвечает на человеческом языке с ссылками на код
    
    answer: String,
    // Человекочитаемый ответ
    
    supporting_evidence: Vec<CodeLocation>,
    // Ссылки на код, подтверждающие ответ
    
    tool_calls_made: Vec<ToolCall>,
    // Какие инструменты использованы (для transparency)
    
    suggested_actions: Vec<Action>,
    // "Рекомендую запустить тест X"
    // "Возможно стоит добавить проверку Y"
    
    related_questions: Vec<String>,
    // "Вас также может интересовать..."
    
    confidence: f32,
    // Уверенность в ответе (0.0-1.0)
}

struct AskContext {
    current_file: Option<String>,
    recent_changes: bool,         // Учитывать uncommitted changes
    include_history: bool,        // Смотреть git history
    cross_project: bool,          // Искать в других проектах
}

// Примеры вопросов (NL → tool orchestration):

"Где может быть race condition?" 
  → search("mutex lock unlock concurrent")
  → find_all_unsafe()
  → analyze thread safety

"Покажи все места где используется небезопасный код"
  → rust_find_unsafe_blocks()
  → get_references for each unsafe block
  → check if safety documented

"Что нужно обновить если я поменяю эту структуру?"
  → get_callers(current_symbol)
  → analyze_uncommitted_changes()
  → suggest_migration_path()

"Как оптимизировать эту функцию?"
  → get_performance_profile(function)
  → find_similar_code (better implementations)
  → suggest_improvements()

"Почему этот тест падает?"
  → get_test_details(test_name)
  → get_recent_changes affecting test
  → analyze_execution_traces
  → find_similar_failures in history
```

**Use Cases:**

```
Сценарий 1: Debugging
User: "Почему parse_file падает на больших файлах?"
gofer: *orchestrates: get_errors(), search("parse_file panic"), analyze_code()*
→ "Найдено 2 потенциальные причины:
   
   1. Stack overflow в рекурсивном парсере (src/parser.rs:234)
      - Решение: ограничить глубину рекурсии
      - Похожий баг был в commit abc123, исправлен через iterative parsing
   
   2. Memory exhaustion на файлах >2MB (pipeline.rs:MAX_FILE_SIZE)
      - Текущий лимит: 2MB
      - Recommendation: либо увеличить лимит, либо streaming parse
   
   Supporting evidence: [ссылки на код]"

Сценарий 2: Learning
User: "Как работает индексация?"
gofer: *orchestrates: search_by_purpose(), get_callees(), explain_flow()*
→ "Индексация происходит в 5 этапов (pipeline pattern):
   
   1. Scanner (watcher.rs) - находит файлы
   2. Parser (parser.rs) - извлекает symbols
   3. Chunker (chunking.rs) - разбивает на chunks
   4. Embedder (embedder.rs) - генерирует vectors
   5. Writer (pipeline.rs) - сохраняет в LanceDB + SQLite
   
   Entry point: pipeline::run_pipeline()
   Key functions: [ссылки]
   
   Хотите детальнее про какой-то этап?"

Сценарий 3: Code review
User: "Что подозрительного в моих изменениях?"
gofer: *orchestrates: review_uncommitted_changes(), security_audit()*
→ "Проверил ваши изменения:
   
   ✅ Стиль и тесты: OK
   ⚠️ 1 проблема безопасности:
      src/api.rs:67 - SQL query без параметризации (injection risk)
   🔵 2 рекомендации:
      - embedder.rs:123 - можно упростить через map()
      - Дублированная логика с storage.rs:456
   
   Детали: [ссылки на код]"

Сценарий 4: Exploration
User: "С чего начать, чтобы понять как работает поиск?"
gofer: *orchestrates: find_learning_path(), get_related_files()*
→ "Рекомендую изучать в таком порядке:
   
   1. Начните с storage/lance.rs (векторный поиск)
      - Ключевые функции: search(), upsert_chunks()
   
   2. Потом indexer/embedder.rs (как создаются векторы)
      - EmbedderPool - пул для параллельных embeddings
   
   3. Затем daemon/tools.rs (как всё связано)
      - tool_search() - entry point для поиска
   
   4. Опционально: indexer/reranker.rs (улучшение результатов)
   
   В проекте есть 15 тестов по теме 'search', рекомендую посмотреть."
```

**Преимущества:**
- **Снижает порог входа** - не нужно знать MCP API
- **Контекстное понимание** вопросов
- **Orchestration** нескольких инструментов
- **Conversational** - можно задавать follow-up вопросы

**Техническая реализация:**
1. NL → Intent classification (LLM)
2. Intent → Tool orchestration plan
3. Execute tools sequentially/parallel
4. Aggregate results
5. Generate natural language answer (LLM)
6. RAG over indexed code для context
7. Conversation history для follow-ups

**Priority:** 🔥🔥 **Medium**  
**Effort:** High (4-5 недель)  
**Impact:** Делает gofer доступным для всех, не только power users

---

## 📊 Сравнительная таблица приоритетов

| Фича | Impact | Effort | Complexity | Priority | Quick Win |
|------|--------|--------|------------|----------|-----------|
| **Token-Efficient Reading** | 🔥🔥🔥 | Low | Low | **Critical** | ✅ Yes |
| **Real-time Change Impact** | 🔥🔥🔥 | Medium | Medium | **Critical** | ✅ Yes |
| **Semantic Diff** | 🔥🔥 | Medium | Medium | High | ❌ No |
| **Embedding Code Review** | 🔥🔥 | Medium | Medium | High | ❌ No |
| **Performance Profiling** | 🔥🔥 | High | High | Medium | ❌ No |
| **Multi-Repo Context** | 🔥 | High | High | Medium | ❌ No |
| **Natural Language Queries** | 🔥🔥 | High | High | Medium | ❌ No |

---

## 🚀 Quick Wins (Что можно сделать быстро)

### 1. Token-Efficient Reading
**Effort:** 1-2 дня  
**Why quick:**
- Skeleton функция уже существует (`src/indexer/parser/skeleton.rs`)
- Нужно только обернуть в MCP tool
- Минимальная логика

**Implementation checklist:**
```rust
// src/daemon/tools.rs

// Новый MCP tool
"read_file_skeleton" => {
    let file_path = get_required_param!(args, "file")?;
    let skeleton = skeleton::skeletonize_file(&file_path)?;
    json!({ "content": skeleton, "tokens_saved": "~70%" })
}

"read_function_context" => {
    let file_path = get_required_param!(args, "file")?;
    let function_name = get_required_param!(args, "function")?;
    // Extract only this function + its dependencies
}
```

**Impact:** Immediate 3-5× token savings в большинстве сценариев

---

### 2. Analyze Uncommitted Changes
**Effort:** 3-5 дней  
**Why quick:**
- `git_diff` уже реализован
- `get_callers` уже реализован
- Нужно только объединить данные

**Implementation checklist:**
```rust
// Новый MCP tool
"analyze_uncommitted_changes" => {
    let diff = git_diff(unstaged: true)?;
    let changed_symbols = parse_diff_for_symbols(diff);
    
    let impact = ChangeImpact {
        affected_callers: get_callers(changed_symbols),
        test_coverage: check_tests_exist(changed_files),
        risk_level: calculate_risk(affected_callers.len()),
    };
    
    json!(impact)
}
```

**Impact:** Проактивная помощь во время разработки

---

## 💡 Дополнительные микро-фичи

Небольшие улучшения, которые дадут большой UX boost:

### 1. **Streaming Progress для длительных операций**
```rust
// Вместо молчания во время indexing
index_sync() → stream progress updates
  "Scanning files... 120/500"
  "Parsing... 45/120"
  "Embedding... batch 3/15"
```

### 2. **Smart caching с TTL**
```rust
// Кэшировать результаты поиска
search_cached(query: String, ttl_seconds: u64)
  // Если тот же query в пределах TTL → instant result
```

### 3. **Explain mode для инструментов**
```rust
// Каждый tool может объяснить ЧТО он делает
tool_call(..., explain: true) → {
  result: ...,
  explanation: "Этот инструмент использует векторный поиск..."
}
```

### 4. **Cost estimation**
```rust
estimate_cost(operation: Operation) → Cost {
  tokens: 1500,
  time_ms: 250,
  api_calls: 3,
}
// Перед дорогой операцией предупредить пользователя
```

### 5. **Bookmarks / Favorites**
```rust
bookmark_add(location: CodeLocation, note: String)
bookmark_list() → Vec<Bookmark>
// Быстрый возврат к важным местам в коде
```

---

## 🎯 Roadmap Integration

Эти фичи **дополняют** основной ROADMAP.md, не заменяют:

**Основной ROADMAP фокус:**
- Runtime context (tests, coverage)
- Code evolution (history, churn)
- Human context (ownership, decisions)
- Index quality
- Smart ranking

**Extended ROADMAP фокус:**
- **Token efficiency** (LLM cost optimization)
- **Real-time assistance** (during development)
- **Cross-cutting concerns** (multi-repo, performance)
- **UX improvements** (NL queries, streaming)

**Синергия:**
Многие фичи работают лучше вместе:
- Token-efficient reading + Context bundle = optimal LLM usage
- Real-time change impact + Code evolution = predictive analysis
- Performance profiling + Runtime context = complete behavior picture

---

## 📝 Feedback & Contribution

**Обсуждение приоритетов:**
Открыт для feedback! Если у вас есть реальный use case для какой-то из фич:
1. Опишите сценарий в issue
2. Мы обсудим приоритет
3. Начнем implementation

**Community input:**
- Какие фичи наиболее ценны для вашего workflow?
- Есть ли другие pain points, которые не покрыты?
- Готовы ли помочь с implementation?

---

## 🔗 Связь с основным ROADMAP

| Extended Roadmap | Main ROADMAP | Relationship |
|------------------|--------------|--------------|
| Token-Efficient Reading | (new) | Enables all other features to scale |
| Real-time Change Impact | Code Evolution | Real-time extension |
| Semantic Diff | Code Evolution | Enhanced diffing |
| Multi-Repo Context | (new) | Team-level scaling |
| Code Review | Human Context | Automated assistance |
| Performance Profiling | Runtime Context | Performance dimension |
| NL Queries | (new) | UX enhancement layer |

---

**Status:** RFC - Request for Comments  
**Next Steps:** 
1. Community feedback на приоритеты
2. Proof-of-concept для Quick Wins
3. Детальные specs для выбранных фич
4. Incremental implementation

**Let's make gofer even more powerful together!** 🚀
