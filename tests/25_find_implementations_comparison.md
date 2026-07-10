# find_implementations vs grep

## Goal
Find types that implement a trait / interface / base class.

## Approaches

| Tool | Query style |
|---|---|
| gofer `find_implementations` | whole-token on signatures (`impl Trait for`, `implements`, bases) |
| `rg 'impl .* for'` / `implements Foo` | text; easy false positives (`FooBar`) |

## When gofer wins
- Cross-file trait name without inventing language-specific regex  
- Whole-token matching (`Foo` ≠ `FooBar`)  
- Structured `{type, kind, relation, file, line}`

## When host wins
- Go structural interfaces (not declared)  
- Fresh unindexed code  
- Ad-hoc patterns outside signatures

## Verdict
Prefer **find_implementations** for Rust/TS/Python OOP-ish graphs; use `rg` for exploratory one-offs.
