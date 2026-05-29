# lang-hub — реестр языковых пакетов

`gofer install-lang <name>` качает tree-sitter-грамматику и LSP-конфиг из публичного репозитория [`budka-dev/lang-hub`](https://github.com/budka-dev/lang-hub). Этот документ описывает, что такое lang-hub, формат пакета и как добавить свой язык.

## Зачем

tree-sitter грамматики не вкомпилены в gofer. Вместо этого:

- Бинарь остаётся легковесным (10+ грамматик не тянутся как deps).
- Добавление языка не требует пересборки gofer.
- Можно использовать собственные форки или экспериментальные грамматики.
- Релизы грамматик независимы от релизов gofer.

## Где живёт

- Репо: `https://github.com/budka-dev/lang-hub`
- Манифесты: `https://raw.githubusercontent.com/budka-dev/lang-hub/main/list/<lang>/manifest.toml`
- Бинарные WASM: `https://github.com/budka-dev/lang-hub/releases/download/latest/tree-sitter-<lang>.wasm`
- Tree-sitter запросы: `https://raw.githubusercontent.com/budka-dev/lang-hub/main/list/<lang>/queries/<query>.scm`
- Tool-манифесты (LSP, форматтеры): `https://raw.githubusercontent.com/budka-dev/lang-hub/main/tools/<tool>/manifest.toml`

URL'ы захардкожены в `src/indexer/parser/lang_manager/mod.rs::install_from_github` (строки 392+).

## Что качается

`gofer install-lang rust`:

1. Скачивает `list/rust/manifest.toml` — описание языка.
2. Скачивает `releases/latest/tree-sitter-rust.wasm` — сам парсер. Если в релизе нет, читает `parser.download_url` из манифеста и качает оттуда.
3. Скачивает девять стандартных query-файлов: `symbols.scm`, `references.scm`, `highlights.scm`, `locals.scm`, `injections.scm`, `folds.scm`, `tags.scm`, `indents.scm`, `outline.scm`. Если файла нет — пропускает.
4. Кладёт всё в `~/.gofer/langs/rust/`:
   ```
   ~/.gofer/langs/rust/
   ├── manifest.toml
   ├── rust.wasm
   └── queries/
       ├── symbols.scm
       ├── references.scm
       └── ...
   ```
5. Загружает пакет (`load_language_from_disk`). После этого язык доступен для индексации.

## Auto-download

Если при индексации попадается файл с расширением неизвестного языка, gofer сам пробует подтянуть пакет из lang-hub (`mod.rs:279`: «Language not found locally, auto-downloading from lang-hub...»). Это удобно, когда переключаешься между проектами и не хочется руками ставить каждую грамматику.

Если автоскачивание не сработало (нет сети, языка нет в lang-hub) — gofer молча пропускает файлы такого типа.

## Формат `manifest.toml`

Структура полностью описана в `src/indexer/parser/lang_manager/mod.rs::LangManifest`. Пример:

```toml
[language]
name = "rust"
extensions = ["rs"]
aliases = ["rs"]              # дополнительные имена для матчинга
root_markers = ["Cargo.toml"] # файлы-маркеры корня проекта на этом языке
[language.comments]
line = "//"
block = ["/*", "*/"]
doc = "///"

[parser]
type = "tree-sitter"
source = "https://github.com/tree-sitter/tree-sitter-rust"
repo = "tree-sitter/tree-sitter-rust"
download_url = "https://github.com/tree-sitter/tree-sitter-rust/releases/download/v0.21.0/tree-sitter-rust.wasm"

[lsp]
name = "rust-analyzer"
command = "rust-analyzer"
args = []
# Альтернативный путь — указать tool из tools/:
# tool = "rust-analyzer"
# download_url = "..."

[sandbox]
compile_cmd = "rustc"
run_cmd = ""
package_manager = "cargo"
test = ["cargo", "test"]
build = ["cargo", "build"]
run = ["cargo", "run"]
script = []
repl = []

[indexer]
ignore_folders = ["target", "Cargo.lock"]

[formatter]
tool = "rustfmt"
args = []

[linter]
tool = "cargo"
args = ["clippy"]
```

Секции `lsp`, `sandbox`, `indexer`, `formatter`, `linter` опциональны.

## Tool-манифесты (LSP, форматтеры)

Если в `[lsp]` указано `tool = "rust-analyzer"`, gofer полезет дополнительно в `tools/rust-analyzer/manifest.toml`:

```toml
[tool]
name = "rust-analyzer"
description = "Rust Language Server"
capabilities = ["lsp", "diagnostics", "completion"]
[tool.install.binary]
url = "https://github.com/rust-lang/rust-analyzer/releases/download/2024-10-21/rust-analyzer-x86_64-unknown-linux-gnu.gz"
executable_name = "rust-analyzer"

[lsp]
command = "rust-analyzer"
args = []

[formatter]
command = "rustfmt"
args = []

[linter]
command = "cargo"
args = ["clippy"]
```

Бинарь, если указан, складывается в `~/.gofer/tools/<name>/`.

## Запросы tree-sitter

Файлы `queries/*.scm` — это S-expression-патерны для tree-sitter. gofer ожидает стандартный набор:

| Файл | Зачем |
|---|---|
| `symbols.scm` | Извлечение функций/структур/классов для таблицы `symbols`. |
| `references.scm` | Граф вызовов (`symbol_references`). |
| `highlights.scm` | Подсветка синтаксиса (если когда-то понадобится). |
| `locals.scm` | Скоупы для определения локальных переменных. |
| `injections.scm` | Вложенные языки (SQL в Rust макросе, HTML в JS, …). |
| `folds.scm` | Свёртка блоков (для будущей IDE-интеграции). |
| `tags.scm` | Универсальные теги (ctags-стиль). |
| `indents.scm` | Правила отступов. |
| `outline.scm` | Дерево outline (для `lsp_document_symbols`). |

В минимальной поставке достаточно `symbols.scm` — остальные опциональны.

## Добавить свой язык

Если хочешь не дожидаться апстрима в lang-hub, а добавить язык локально:

1. Создай `~/.gofer/langs/mylang/manifest.toml` по образцу выше.
2. Скачай или собери `mylang.wasm` (tree-sitter wasm-target) и положи рядом.
3. Положи хотя бы `queries/symbols.scm`.
4. Перезапусти демон или вызови `gofer install-lang mylang` (он переустановит, но при упавшей сети упадёт — для локального теста проще убрать сетевой шаг и руками положить файлы).

Демон подхватит язык лениво — при первом файле с подходящим расширением.

## Сборка собственного `.wasm`

```bash
git clone https://github.com/tree-sitter/tree-sitter-mylang
cd tree-sitter-mylang
tree-sitter build --wasm --output mylang.wasm
cp mylang.wasm ~/.gofer/langs/mylang/
```

Требует установленный `tree-sitter-cli` и Emscripten SDK для wasm-target.

## Совместимость ABI

tree-sitter wasm имеет внутреннюю ABI-версию. gofer использует `tree-sitter 0.26.7` (см. `Cargo.toml`). Грамматики, собранные для несовместимых ABI, отказывают при загрузке.

В логе появится:

```
WARN gofer::parser: wasm ABI version mismatch for <lang>, skipping
```

Это **не ошибка** — индексация продолжится, просто файлы этого языка не парсятся. См. коммит `d99d62e` (`test(parser): gracefully handle wasm ABI version mismatch`).

Что делать: пересобрать грамматику с совместимым tree-sitter или подождать апдейта lang-hub.

## Где смотреть в коде

- `src/indexer/parser/lang_manager/mod.rs::LangManifest` — структура манифеста.
- `src/indexer/parser/lang_manager/mod.rs::install_from_github` — логика скачивания.
- `src/indexer/parser/lang_manager/mod.rs::load_language_from_disk` — подключение пакета.
- `src/main.rs::handle_install_lang` — CLI-команда.
