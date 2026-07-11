#!/usr/bin/env bash
# Golden-query self-bench against a live gofer daemon (gofer project itself).
#
# Requires: gofer on PATH, healthy daemon, project indexed (run from repo root).
# Hard-fail: binary missing, unhealthy daemon, or search crash/non-zero exit.
# Soft-fail: first hit path misses expected substring (warn only).
#
# Usage:
#   ./scripts/self_bench.sh
#   LIMIT=10 ./scripts/self_bench.sh
#
# See also: ./scripts/bench_live.sh [query]  (single-query micro-bench)

set -euo pipefail

LIMIT="${LIMIT:-5}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v gofer >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/gofer" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v gofer >/dev/null 2>&1; then
  echo "error: gofer binary not found in PATH" >&2
  exit 1
fi

echo "== self_bench (cwd=$ROOT, limit=$LIMIT) =="
echo "== health =="
if ! HEALTH_OUT=$(gofer health 2>&1); then
  echo "error: gofer health failed" >&2
  echo "$HEALTH_OUT" >&2
  exit 1
fi
echo "$HEALTH_OUT"
if ! echo "$HEALTH_OUT" | grep -qiE '"status"[[:space:]]*:[[:space:]]*"healthy"|healthy'; then
  echo "error: daemon not healthy (expected status healthy)" >&2
  exit 1
fi
echo

# query|expected_path_substrings (pipe-separated alternatives; empty = no soft check)
# Soft-check only when search output is JSON-parseable.
GOLDEN=(
  "dispatch|tools.rs|server"
  "tool_search|search.rs"
  "resolve_references|"
  "skeleton|"
  "full_sync|"
)

WARN=0
FAIL=0
N=0

check_first_hit() {
  local out="$1"
  shift
  local -a expect=("$@")
  # No expected substrings → skip soft check.
  if [[ ${#expect[@]} -eq 0 || -z "${expect[0]:-}" ]]; then
    return 0
  fi

  if ! command -v python3 >/dev/null 2>&1; then
    echo "  soft: skip path check (python3 not available)"
    return 0
  fi

  local first
  first=$(python3 -c '
import json, sys
raw = sys.stdin.read()
try:
    data = json.loads(raw)
except Exception:
    sys.exit(2)
results = data.get("results") or data.get("result", {}).get("results") or []
if not results:
    sys.exit(3)
hit = results[0]
if isinstance(hit, dict):
    path = hit.get("file") or hit.get("path") or hit.get("file_path") or ""
elif isinstance(hit, str):
    # CLI format: "path:line (ctx:...)\\ncontent" or "path:line\\n..."
    path = hit.split("\n", 1)[0].split(":", 1)[0]
else:
    path = str(hit)
print(path)
' <<<"$out" 2>/dev/null) || {
    local ec=$?
    if [[ $ec -eq 2 ]]; then
      echo "  soft: output not JSON-parseable — skip path check"
      return 0
    fi
    if [[ $ec -eq 3 ]]; then
      echo "  soft: WARN empty results (no first hit)"
      return 1
    fi
    echo "  soft: WARN could not extract first hit path"
    return 1
  }

  local alt
  for alt in "${expect[@]}"; do
    [[ -z "$alt" ]] && continue
    if [[ "$first" == *"$alt"* ]]; then
      echo "  soft: ok first_hit contains '$alt' (path=$first)"
      return 0
    fi
  done
  local joined
  joined=$(IFS='|'; echo "${expect[*]}")
  echo "  soft: WARN first_hit path '$first' missing expected [$joined]"
  return 1
}

for entry in "${GOLDEN[@]}"; do
  IFS='|' read -r query rest <<<"$entry"
  IFS='|' read -r -a expect <<<"${rest:-}"

  N=$((N + 1))
  echo "== [$N] search: $query (limit=$LIMIT) =="

  START=$(date +%s%3N)
  set +e
  OUT=$(gofer search "$query" -l "$LIMIT" 2>&1)
  RC=$?
  set -e
  END=$(date +%s%3N)
  WALL=$((END - START))

  if [[ $RC -ne 0 ]]; then
    echo "  HARD-FAIL: gofer search exited $RC"
    echo "$OUT" | head -c 1500
    echo
    echo "  wall_ms=$WALL"
    FAIL=$((FAIL + 1))
    continue
  fi

  # Truncate noisy dumps for terminal readability.
  echo "$OUT" | head -c 1200
  if [[ ${#OUT} -gt 1200 ]]; then
    echo
    echo "  … (${#OUT} bytes total)"
  fi
  echo
  echo "  wall_ms=$WALL rc=$RC"

  if ! check_first_hit "$OUT" "${expect[@]+"${expect[@]}"}"; then
    WARN=$((WARN + 1))
  fi
  echo
done

echo "== summary =="
echo "queries=$N hard_fail=$FAIL soft_warn=$WARN"
if [[ $FAIL -ne 0 ]]; then
  echo "result: FAIL (search crash or non-zero exit)"
  exit 1
fi
echo "result: OK (soft warnings do not fail the run)"
exit 0
