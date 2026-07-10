# Сравнительные отчёты

Этот каталог содержит **не классические unit-тесты**, а развёрнутые сравнения MCP-инструментов gofer с native-аналогами (`cat`, `grep`, `find`, `glob`, чтение целым файлом). Это методологически зафиксированные замеры эффективности, которые показывают, **где gofer экономит токены и время**, а где он эквивалентен или хуже.

Юнит-тесты живут как `#[cfg(test)] mod tests` внутри `src/` — см. [docs/development.md::Тесты](../docs/development.md#тесты).

## Методология

[00_methodology.md](00_methodology.md) — единые метрики для всех отчётов: функциональность, точность (Accuracy 0–100%), эффективность по токенам и latency. Читай её перед тем, как смотреть конкретные сравнения.

## Каталог

### Чтение файлов

| № | Файл | Что сравнивается |
|---|---|---|
| 01 | [read_file_comparison.md](01_read_file_comparison.md) | `read_file` vs `cat` |
| 03 | [skeleton_vs_read_comparison.md](03_skeleton_vs_read_comparison.md) | `skeleton` vs полный `read_file` |
| 13 | [context_bundle_comparison.md](13_context_bundle_comparison.md) | `context_bundle` vs ручное собирание импортов |
| 14 | [get_file_metadata_comparison.md](14_get_file_metadata_comparison.md) | `get_file_metadata` vs `ls -la` + `wc -l` + `file` |
| 15 | [read_function_context_comparison.md](15_read_function_context_comparison.md) | `read_function_context` vs полный `read_file` |
| 16 | [read_types_only_comparison.md](16_read_types_only_comparison.md) | `read_types_only` vs чтение целиком |

### Поиск

| № | Файл | Что сравнивается |
|---|---|---|
| 02 | [search_vs_grep_comparison.md](02_search_vs_grep_comparison.md) | Семантический `search` vs `grep`/`rg` |
| 04 | [get_symbols_vs_grep_comparison.md](04_get_symbols_vs_grep_comparison.md) | `get_symbols` vs `grep -n "fn \\|struct \\|impl "` |
| 06 | [find_files_vs_glob_comparison.md](06_find_files_vs_glob_comparison.md) | `find_files` vs системный `glob`/`find` |
| 08 | [grep_comparison.md](08_grep_comparison.md) | gofer'овский `grep` vs ripgrep |
| 09 | [project_tree_comparison.md](09_project_tree_comparison.md) | `project_tree` vs `tree`/`ls -R` |
| 11 | [list_directory_comparison.md](11_list_directory_comparison.md) | `list_directory` vs `ls` |
| 17 | [smart_file_selection_comparison.md](17_smart_file_selection_comparison.md) | `smart_file_selection` vs ручной grep + интуиция |
| 18 | [search_symbols_comparison.md](18_search_symbols_comparison.md) | `search_symbols` vs `grep -n` + ctags |
| 19 | [file_exists_comparison.md](19_file_exists_comparison.md) | `file_exists` vs `test -f` |

### Изменение файлов

| № | Файл | Что сравнивается |
|---|---|---|
| 05 | [patch_file_vs_edit_comparison.md](05_patch_file_vs_edit_comparison.md) | `patch_file` (search & replace) vs полная перезапись |
| 10 | [write_file_comparison.md](10_write_file_comparison.md) | `write_file` vs запись через native shell |
| 12 | [move_file_comparison.md](12_move_file_comparison.md) | `move_file` vs `mv` |

### Composite-операции

| № | Файл | Что сравнивается |
|---|---|---|
| 07 | [batch_operations_comparison.md](07_batch_operations_comparison.md) | `batch_operations` (N запросов в одном) vs последовательные вызовы |
| 20 | [transactions_comparison.md](20_transactions_comparison.md) | Транзакции gofer vs цепочка ручных операций |
| 21 | [hash_buffers_comparison.md](21_hash_buffers_comparison.md) | CAS-буфер (`clipboard_*`) vs повторная передача кода |

### Sandbox

| № | Файл | Что сравнивается |
|---|---|---|
| 22 | [execute_sandbox_comparison.md](22_execute_sandbox_comparison.md) | `execute_code`/`execute_function` vs ручное `cargo run`/`python` |

## Чего здесь нет

- LSP-инструменты (`lsp_*`) не покрыты сравнениями — слишком разные с native-аналогами по природе.
- Lang-tools (`rust_*`, `vue_*`, etc.) — то же самое.
- Git-инструменты (`git_diff`, `git_blame`, `git_history`, `suggest_commit`) — пока без отдельных отчётов.

## Как использовать

Если делаешь PR с оптимизацией существующего инструмента или добавляешь новый — приложи свежий замер в формате `tests/NN_<tool>_comparison.md` по [методологии](00_methodology.md). Это лучшая форма ревью-аргумента.

Если просто хочешь понять, **где gofer быстрее/дешевле**, чем native — это полезный материал перед интеграцией.

> **2026-07:** MCP surface is ~16 index-search tools only. Comparison docs that mention `grep`, `read_file`, `write_file`, `patch_file`, git, or analysis tools are historical; host agent owns those. Prefer scenarios for `search`, `skeleton`, `get_references` / callers, `context_bundle`.
