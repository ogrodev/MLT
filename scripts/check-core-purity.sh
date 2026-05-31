#!/usr/bin/env bash
# Architecture-fitness gate (docs/QUALITY_GATES.md §2, ADR 0006):
# `crates/core` must stay PURE — no IO-performing dependency and no direct OS/clock/net/fs
# calls. Side effects may only enter through `core::ports`. This makes the invariant
# machine-enforced rather than honor-system. Run from the repo root.
set -euo pipefail

cd "$(dirname "$0")/.."
fail=0

echo "→ checking crates/core dependency tree (normal deps only)…"
# Dev-deps (e.g. the tokio test runtime) are intentionally excluded via --edges normal.
deps=$(cargo tree -p mlt-core --edges normal --prefix none 2>/dev/null \
  | sed 's/ .*//' | sort -u)
for crate in reqwest hyper sqlx rusqlite diesel keyring dirs ureq surf isahc; do
  if printf '%s\n' "$deps" | grep -qx "$crate"; then
    echo "  ✗ FORBIDDEN IO crate in core deps: $crate"
    fail=1
  fi
done
[ "$fail" -eq 0 ] && echo "  ✓ no IO crates in core's normal deps"

echo "→ checking crates/core source for forbidden APIs…"
# Time must come from the Clock port; no direct fs/net/http in core. Comment/doc lines
# (which legitimately mention these APIs to explain the rule) are excluded.
matches=$(grep -RnE \
  'SystemTime::now|Instant::now|std::fs::|std::net::|std::process::|reqwest::|tokio::(net|fs)' \
  crates/core/src --include='*.rs' \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*(//|/\*|\*)' || true)
if [ -n "$matches" ]; then
  echo "  ✗ FORBIDDEN api usage in core:"
  printf '%s\n' "$matches" | sed 's/^/      /'
  fail=1
else
  echo "  ✓ no forbidden OS/clock/net/fs calls in core source"
fi

if [ "$fail" -ne 0 ]; then
  echo "core-purity: FAILED"
  exit 1
fi
echo "core-purity: OK"
