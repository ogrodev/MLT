#!/usr/bin/env bash
# Machine-readable backlog of NOT-DONE tasks, for tooling (the /mlt:start-task command).
# One TSV row per task, already ordered by number:
#
#   num <TAB> slug <TAB> status <TAB> checked <TAB> total <TAB> title <TAB> path
#
# "Not done" mirrors check-tasks.sh exactly: a task counts as done only when its Status is
# "done" AND every acceptance criterion is checked. Always exits 0 — this is a data source,
# not a gate. Run from anywhere; resolves the repo root itself.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=scripts/lib/tasks.sh
source scripts/lib/tasks.sh

for f in docs/tasks/[0-9]*.md; do
  [ -e "$f" ] || continue
  base=$(basename "$f" .md)
  num=${base%%-*}
  slug=${base#*-}

  IFS=$'\t' read -r status checked unchecked < <(mlt_task_parse <"$f")
  total=$((checked + unchecked))

  # Skip finished tasks (status "done" with every criterion ticked) — same rule as check-tasks.
  if [ "$status" = "done" ] && { [ "$total" -eq 0 ] || [ "$checked" -eq "$total" ]; }; then
    continue
  fi

  title=$(mlt_task_title <"$f")
  [ -n "$title" ] || title="$base"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$num" "$slug" "$status" "$checked" "$total" "$title" "$f"
done
