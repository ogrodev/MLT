#!/usr/bin/env bash
# Claude Code PostToolUse hook: auto-format Rust files right after Claude edits them.
# Non-blocking and defensive — it must never fail or produce noise.
set -uo pipefail
input="$(cat 2>/dev/null || true)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null || true)"
case "${file:-}" in
  *.rs)
    rustbin="$(dirname "$(realpath "$(command -v rustc 2>/dev/null)" 2>/dev/null)" 2>/dev/null || true)"
    if [ -n "${rustbin:-}" ] && [ -x "$rustbin/rustfmt" ] && [ -f "$file" ]; then
      "$rustbin/rustfmt" --edition 2021 "$file" >/dev/null 2>&1 || true
    fi
    ;;
esac
exit 0
