#!/usr/bin/env bash
# Shared task-doc parser for docs/tasks/NNN-*.md. Sourced by check-tasks.sh (the human/CI
# status table), task-list.sh (machine-readable backlog for the /mlt:start-task command), and
# check-task-started.sh (the pre-commit task gate) so all three read a task's Status and
# acceptance-criteria checkboxes the SAME way — one source of truth, no drift.
#
# Not meant to run on its own; `source` it after setting your own shell options.

# mlt_task_parse — read a task doc on stdin, print "<status>\t<checked>\t<unchecked>".
#   status   : the **Status:** label with its leading emoji stripped — "not started" / "partial"
#              / "done" / "missing" (when no Status line is present).
#   checked  : count of ticked acceptance criteria   (- [x])
#   unchecked: count of un-ticked acceptance criteria (- [ ])
mlt_task_parse() {
  local content status checked unchecked
  content=$(cat)
  # Text after "**Status:**", trimmed to the first " ·" field separator, emoji stripped.
  status=$( { printf '%s\n' "$content" | grep -m1 '\*\*Status:\*\*' || true; } \
    | sed -E 's/.*\*\*Status:\*\*[[:space:]]*//; s/[[:space:]]*·.*//' \
    | sed -E 's/^[^[:alnum:]]+[[:space:]]*//' \
    | sed -E 's/[[:space:]]+$//')
  [ -n "$status" ] || status="missing"
  # Acceptance-criteria checkboxes anywhere in the doc.
  checked=$(printf '%s\n' "$content" | grep -ciE '^[[:space:]]*- \[x\]' || true)
  unchecked=$(printf '%s\n' "$content" | grep -ciE '^[[:space:]]*- \[ \]' || true)
  printf '%s\t%s\t%s\n' "$status" "$checked" "$unchecked"
}

# mlt_task_title — read a task doc on stdin, print its H1 title text (e.g. "005 — Codex usage").
mlt_task_title() {
  { grep -m1 '^# ' || true; } | sed -E 's/^#[[:space:]]+//; s/[[:space:]]+$//'
}
