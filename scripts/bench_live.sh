#!/usr/bin/env bash
# Live micro-bench against a running gofer daemon (current project cwd).
# Single query. For multi golden-query self-bench, see ./scripts/self_bench.sh
# Usage: ./scripts/bench_live.sh [query]
set -euo pipefail

QUERY="${1:-dispatch}"
LIMIT="${LIMIT:-5}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${PROJECT_DIR:-$ROOT}"

if ! command -v gofer >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/gofer" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v gofer >/dev/null 2>&1; then
  echo "gofer binary not found in PATH" >&2
  exit 1
fi

echo "== health =="
gofer health || true
echo
echo "== status (snippet) =="
gofer status || true
echo
echo "== search: $QUERY (limit=$LIMIT) =="
START=$(date +%s%3N)
OUT=$(gofer search "$QUERY" -l "$LIMIT" 2>&1) || true
END=$(date +%s%3N)
echo "$OUT" | head -c 2000
echo
echo "wall_ms=$((END - START))"
echo
echo "== tip =="
echo "Re-run after: gofer reindex --force   (or MCP reindex force=true)"
echo "Compare host: rg -n '$QUERY' | head"
echo "Golden queries: ./scripts/self_bench.sh"
