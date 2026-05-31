#!/usr/bin/env bash
# Task-status validator (docs/tasks/): at a glance, which tasks are done and which aren't.
#
# Each task doc (docs/tasks/NNN-*.md) declares a **Status:** (◻ not started / 🟡 partial /
# ✅ done) and a list of acceptance-criteria checkboxes (- [ ] / - [x]). This keeps the two
# honest with each other and gives a one-screen overview:
#   • prints a table of every task with its status and N/M criteria checked,
#   • FAILS (non-zero) on a contradiction — marked "done" with unchecked criteria, or every
#     criterion checked but not marked "done" — so a stale Status line can't slip by.
#
# Usage: scripts/check-tasks.sh            # full table + summary, exits 1 on any mismatch
#        scripts/check-tasks.sh --todo     # only tasks that aren't done yet
#        scripts/check-tasks.sh --done     # only finished tasks
# Run from anywhere; resolves the repo root itself.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=scripts/lib/tasks.sh
source scripts/lib/tasks.sh
tasks_dir="docs/tasks"

filter="all"
case "${1:-}" in
  --todo) filter="todo" ;;
  --done) filter="done" ;;
  --all|"") filter="all" ;;
  -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "unknown option: $1 (try --todo, --done, or --help)" >&2; exit 2 ;;
esac

n_done=0 n_progress=0 n_todo=0 n_mismatch=0
mismatches=()

printf '  %-3s  %-26s  %-12s  %-9s  %s\n' "#" "task" "status" "criteria" "verdict"
printf '  %-3s  %-26s  %-12s  %-9s  %s\n' "---" "--------------------------" "------------" "---------" "-------"

for f in "$tasks_dir"/[0-9]*.md; do
  [ -e "$f" ] || continue
  base=$(basename "$f" .md)
  num=${base%%-*}
  slug=${base#*-}

  # Status + acceptance-criteria counts come from the shared parser (scripts/lib/tasks.sh)
  # so this table and the pre-commit task gate read every task doc identically.
  IFS=$'\t' read -r status checked unchecked < <(mlt_task_parse <"$f")
  total=$((checked + unchecked))

  # Derive the verdict by cross-checking the declared status against the checkboxes.
  is_done="no"
  case "$status" in
    done)
      if [ "$total" -gt 0 ] && [ "$checked" -lt "$total" ]; then
        verdict="⚠ MISMATCH"
        mismatches+=("$num $slug: marked \"done\" but $((total - checked))/$total criteria unchecked")
        n_mismatch=$((n_mismatch + 1))
      else
        verdict="✅ done"; is_done="yes"; n_done=$((n_done + 1))
      fi
      ;;
    *)
      if [ "$total" -gt 0 ] && [ "$checked" -eq "$total" ]; then
        verdict="⚠ MISMATCH"
        mismatches+=("$num $slug: all $total criteria checked but status is \"$status\", not done")
        n_mismatch=$((n_mismatch + 1))
      elif [ "$status" = "missing" ]; then
        verdict="⚠ no status"
        mismatches+=("$num $slug: no **Status:** line found")
        n_mismatch=$((n_mismatch + 1))
      elif [ "$checked" -gt 0 ]; then
        verdict="🟡 in progress"; n_progress=$((n_progress + 1))
      else
        verdict="◻ todo"; n_todo=$((n_todo + 1))
      fi
      ;;
  esac

  # Apply the view filter (mismatches always show — they need attention).
  if [ "$filter" = "done" ] && [ "$is_done" != "yes" ]; then continue; fi
  if [ "$filter" = "todo" ] && [ "$is_done" = "yes" ]; then continue; fi

  printf '  %-3s  %-26.26s  %-12s  %4s/%-4s %s\n' \
    "$num" "$slug" "$status" "$checked" "$total" "$verdict"
done

total_tasks=$((n_done + n_progress + n_todo + n_mismatch))
echo
echo "  $total_tasks tasks: $n_done done · $n_progress in progress · $n_todo todo · $n_mismatch needing attention"

if [ "${#mismatches[@]}" -gt 0 ]; then
  echo
  echo "  status/criteria mismatches — fix the **Status:** line or the checkboxes:"
  for m in "${mismatches[@]}"; do
    echo "    ✗ $m"
  done
  echo
  echo "check-tasks: FAILED"
  exit 1
fi

echo
echo "check-tasks: OK"
