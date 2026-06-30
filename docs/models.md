# Модели данных индекса

Что лежит в SQLite/LanceDB после индексации. Полезно для тех, кто пишет handler'ы, лазает в БД напрямую или хочет понять, какие поля доступны.

Реализация — `src/models/chunk.rs`. Все структуры производны `Serialize`/`Deserialize`; те, что хранятся в SQLite, — также `sqlx::FromRow`; те, что в горячем индексе или LanceDB, — `Archive`/`RkyvSerialize` для zero-copy.

## Файл

```rust
struct IndexedFile {
    id: i64,                    // первичный ключ
    path: String,               // абсолютный путь
    last_modified: i64,         // mtime (UNIX seconds)
    content_hash: String,       // blake3 содержимого
}
```

Хранится в `files`. Используется для skip-unchanged: если `content_hash` совпадает с тем, что в БД, файл не переиндексируется.

В таблице `files` есть ещё колонки, добавляемые миграциями: `domain`, `tech_stack` (миграция `001`), `indexing_status` ∈ `{pending, in_progress, completed, failed}`, `language` (миграция `015`). Они доступны через handler'ы (`get_index_status`), но не присутствуют в самой структуре `IndexedFile`.

## Символ

```rust
struct Symbol {
    id: i64,
    file_id: i64,               // FK на IndexedFile.id
    name: String,
    kind: SymbolKind,
    line_start: i32,            // 1-indexed
    line_end: i32,
    signature: Option<String>,  // полная сигнатура (для функций — с возвращаемым типом)
}
```

Для MCP-инструментов также используется `SymbolWithPath`, где вместо `file_id` лежит готовый `file_path` (плоский ответ без джойнов).

### `SymbolKind`

Закрытое перечисление. SQLite хранит как TEXT (`as_str()`/`from_str()` — взаимная конвертация):

| Вариант | Строковое | Что означает |
|---|---|---|
| `Function` | `function` / `fn` (alias на чтение) | Свободная функция. |
| `Method` | `method` | Метод impl-блока или класса. |
| `Struct` | `struct` | Структура / класс с полями (Rust, Go). |
| `Class` | `class` | Класс (Python, TS, Vue). |
| `Enum` | `enum` | Перечисление. |
| `Impl` | `impl` | Блок `impl` (Rust). |
| `Trait` | `trait` | Trait (Rust). |
| `Interface` | `interface` | Интерфейс (TS, Go). |
| `Const` | `const` | Константа. |
| `Type` | `type` | Базовый тип (в Go, например). |
| `TypeAlias` | `type_alias` | Алиас типа. |
| `Module` | `module` / `mod` (alias) | Модуль/неймспейс. |
| `LocalVar` | `local_var` | Локальная переменная (редко индексируется). |

Default при неизвестной строке — `Function`.

## Ссылка на символ (граф вызовов)

```rust
struct SymbolReference {
    id: i64,
    source_symbol_id: i64,             // кто ссылается
    target_name: String,               // на кого (по имени)
    target_symbol_id: Option<i64>,     // если разрешено — id цели
    kind: String,                      // см. ReferenceKind ниже
    line: i32,
}

enum ReferenceKind {
    Call,       // вызов функции/метода
    Import,     // use / import statement
    TypeUsage,  // использование типа в аннотации
    Inherit,    // impl ... for ..., extends, implements
}
```

`target_symbol_id` может быть `None`, если ссылка не разрешилась (внешняя зависимость, ambiguity). Тогда `target_name` — единственная зацепка.

Для MCP плоский вариант — `ReferenceWithPath` (без `source_symbol_id`/`target_symbol_id`, с готовым `file_path`).

## Чанк (для векторного поиска)

```rust
struct CodeChunk {
    id: String,                     // уникальный (file:start-end или хеш)
    file_path: String,
    content: String,                // сырой текст чанка
    line_start: u32,
    line_end: u32,
    symbol_name: Option<String>,    // если чанк = функция/класс
    symbol_kind: Option<SymbolKind>,
    symbol_path: Option<String>,    // "Service::method" или "mod::fn"
    scopes: Vec<String>,            // стек скоупов для oversized-чанков
}
```

Хранится в LanceDB вместе с embedding-вектором (поле `embedding: FixedSizeList<f32, dimensions>`). `id` используется как ключ во вторичных запросах.

## Результат поиска

```rust
struct SearchResult {
    file_path: String,
    content: String,
    line_start: u32,
    line_end: u32,
    score: f32,         // 0.0..=1.0, выше — релевантнее
    match_type: MatchType,
}

enum MatchType {
    Semantic,    // через эмбеддинг
    Keyword,     // через BM25 / FTS
    Hybrid,      // оба сошлись
}
```

`score` нормализован: для семантического поиска — cosine similarity, для FTS — нормализованный BM25. В hybrid берётся взвешенное среднее.

## Импорт

```rust
struct ImportInfo {
    path: String,         // "./components/Button" или "lodash"
    items: Vec<String>,   // ["Button", "ButtonProps"] или ["default"]
    is_relative: bool,    // true для локальных
    line: u32,
}
```

Извлекается tree-sitter'ом на этапе parse. Используется `context_bundle` для построения зависимого графа и `dependency_impact` для backward-поиска.

## Context bundle

```rust
struct ContextBundle {
    main_file: String,
    main_content: String,
    dependencies: Vec<DependencyFile>,
    markdown: String,             // готовый markdown-блок для LLM
    total_lines: usize,
    total_tokens_estimate: usize, // эвристика (≈ chars / 4)
}

struct DependencyFile {
    path: String,
    content: String,
    reason: String,    // "imported type", "imported component", ...
    depth: u32,
}
```

`markdown` — то, что попадает прямо в ответ инструмента: с заголовками файлов и обозначением, почему они включены.

## Зависимости проекта

```rust
struct Dependency {
    id: i64,
    name: String,
    version: String,
    ecosystem: String,        // "cargo" или "npm"
    features: Option<String>, // JSON-массив включённых features
    dev_only: i32,            // 0/1
    updated_at: i64,
}
```

Парсятся из `Cargo.toml`/`package.json`. `python_read_manifest` отдельно для Python (pyproject.toml/Pipfile/requirements.txt) — там модель другая, возвращается ad-hoc.

```rust
struct DependencyUsage {
    id: i64,
    dependency_id: i64,
    file_id: i64,
    line: i32,
    usage_type: String,       // "use", "import", "require"
    import_path: String,
    items: Option<String>,    // JSON-массив импортированных имён
}
```

Через `dependency_impact` отдаётся `DependencyImpact` со списком `ImpactedFile { path, lines, usage_types }`.

## Правила (rules) и Golden samples

```rust
struct Rule {
    id: i64,
    category: String,    // "general", "naming", "architecture", ...
    rule: String,        // текст правила
    priority: i32,
    source: Option<String>, // "mcp_tool", "config", ...
}

struct GoldenSample {
    id: i64,
    file_id: i64,
    category: Option<String>,
    description: Option<String>,
}
```

Хранятся в SQLite и отдаются через MCP-ресурс `project://context` при чтении. Могут быть заполнены внешними инструментами напрямую через SQLite.

## Конфиг-ключи

```rust
struct ConfigKey {
    id: i64,
    key_name: String,
    data_type: Option<String>,       // "string", "int", "bool"
    source: String,                  // ".env.example", "src/config.rs"
    description: Option<String>,
    default_value: Option<String>,
    required: i32,                   // 0/1
}
```

Заполняется при индексации эвристикой по `.env.example` и Rust-структурам с `#[derive(Deserialize)]`. Отдаётся `get_config_keys`.

## Vue tree

```rust
struct VueTree {
    id: i64,
    file_id: i64,
    tree_text: String,    // готовый текстовый рендер дерева компонента
    components: String,   // JSON-массив дочерних компонентов
    updated_at: i64,
}
```

Заполняется при парсинге `.vue`. Отдаётся `get_vue_tree`.

## API endpoints (cross-stack)

```rust
struct ApiEndpointInfo {
    id: i64,
    method: String,       // "GET", "POST", ...
    path: String,         // "/api/users/:id"
    file_id: i64,
    line: Option<i32>,
}

struct FrontendApiCallInfo {
    id: i64,
    method: Option<String>,
    path: String,
    path_pattern: Option<String>,  // нормализованный для матчинга
    file_id: i64,
    line: Option<i32>,
}
```

Заполняются эвристикой по сигнатурам обработчиков (Rust/Express/FastAPI) и вызовам fetch/axios. Используются `get_api_routes` (handler существует, но не зарегистрирован в dispatch — см. примечание в `tools-reference.md`).

## Structural fingerprinting

Для cross-stack-связей (backend type ↔ frontend type, у которых одинаковые поля).

```rust
struct TypeFingerprint {
    id: i64,
    file_id: i64,
    symbol_id: i64,
    type_name: String,
    language: String,
    fields_json: String,         // оригинальные поля
    fields_normalized: String,   // lower, без сепараторов: "userid" ← "user_id"/"userId"
    field_count: i32,
    file_path: String,
}

struct CrossStackLink {
    id: i64,
    source_file: String,
    target_file: String,
    source_symbol: String,
    target_symbol: String,
    link_type: String,           // "type_match", "endpoint_match"
    weight: f64,                 // 0..1, чем выше — тем уверенее связь
    metadata: Option<String>,    // JSON
    created_at: i64,
}

struct TypeField {
    name: String,
    field_type: Option<String>,
    normalized: String,
}
```

Сравнение через Jaccard по нормализованным полям. Подробности — миграция `007_structural_links.sql`.

## Active errors (компилятор)

```rust
struct ActiveError {
    id: i64,
    file_path: String,
    line: i32,
    column: Option<i32>,
    severity: String,        // "error", "warning"
    code: Option<String>,    // "E0277", "TS2322"
    message: String,
    suggestion: Option<String>,
    updated_at: i64,
}
```

Кеш компиляторных диагностик (таблица сохранена в схеме, но инструменты записи в неё не входят в текущий read-only API).

## ProjectContext

Сводный snapshot для впрыска в LLM (используется ресурсом `project://context`):

```rust
struct ProjectContext {
    dependencies: Vec<Dependency>,
    rules: Vec<Rule>,
    golden_samples: Vec<String>,    // пути файлов
    prompt_fragment: String,        // готовый текст для system prompt
}
```

## Где смотреть в SQLite напрямую

Если хочется покопаться без MCP:

```bash
sqlite3 /path/to/project/.gofer/data/index.sqlite
.tables
.schema symbols
SELECT * FROM symbols WHERE name = 'dispatch' LIMIT 5;
```

WAL-режим включён — открывай в read-only, если демон активен (`?mode=ro` в URI или `PRAGMA query_only=1`).
