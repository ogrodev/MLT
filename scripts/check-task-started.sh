#!/usr/bin/env bash
# Pre-commit task gate (wired into lefthook). When you commit on a task branch — task/NNN-slug,
# created by the /mlt:start-task command — this makes sure the matching docs/tasks/NNN-*.md
# actually moves with the work and stays internally honest:
#
#   V1  the doc's committed content must DIFFER from `main` — i.e. you've reflected progress.
#       Blocks until you tick criteria / set Status and `git add` the doc.
#   V2  the doc must not CONTRADICT itself (same rule check-tasks enforces):
#         • not "✅ done" while acceptance criteria are still unchecked, and
#         • not all-criteria-checked while Status is still "🟡 partial".
#
# It reasons about the *staged* (index) content — exactly what the commit will contain — and
# prints actionable feedback to stderr, exiting non-zero to block. Off a task branch (e.g. a
# plain commit on main) it is a silent no-op. Run from anywhere; resolves the repo root itself.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=scripts/lib/tasks.sh
source scripts/lib/tasks.sh

emit() { printf '%s\n' "$@" >&2; }

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
# Only task/NNN-… branches are gated. Anything else (main, feat/…, detached HEAD) → no-op.
num=$(printf '%s' "$branch" | sed -nE 's#^task/0*([0-9]+)-.*#\1#p')
[ -n "$num" ] || exit 0
printf -v padded '%03d' "$num"

taskfile=""
for f in docs/tasks/"$padded"-*.md; do
  [ -e "$f" ] && { taskfile="$f"; break; }
done
if [ -z "$taskfile" ]; then
  emit "⚠ task-gate: on branch '$branch' but no docs/tasks/$padded-*.md found — skipping."
  exit 0
fi
slug=$(basename "$taskfile" .md); slug=${slug#*-}

# Where this branch left main (fall back to origin/main, then to "no base").
base=$(git merge-base HEAD main 2>/dev/null \
  || git merge-base HEAD origin/main 2>/dev/null \
  || echo "")

# --- V1: the task doc must reflect progress (committed tree differs from main) ---
updated=1
if [ -n "$base" ]; then
  # --cached compares <base> to the index = the tree this commit will write.
  if git diff --cached --quiet "$base" -- "$taskfile"; then updated=0; fi
else
  # No base to diff against — require the doc to be part of this commit.
  git diff --cached --name-only | grep -qxF "$taskfile" || updated=0
fi

if [ "$updated" -eq 0 ]; then
  emit \
    "" \
    "✗ task-gate: task $padded ($slug) — $taskfile is unchanged from main." \
    "" \
    "You're committing work on task $padded but its task doc shows no progress yet. Before committing:" \
    "  • tick the acceptance criteria you've satisfied:  - [ ]  ->  - [x]" \
    "  • set the **Status:** line to 🟡 partial (✅ done only once ALL criteria + the shared" \
    "    Definition of Done are met)" \
    "then stage it:" \
    "      git add $taskfile" \
    "" \
    "Run 'make tasks' to see the current view. Commit blocked."
  exit 1
fi

# --- V2: internal consistency of the committed task doc ---
content=$(git show ":$taskfile" 2>/dev/null || true)
[ -n "$content" ] || content=$(cat "$taskfile" 2>/dev/null || true)
IFS=$'\t' read -r status checked unchecked <<<"$(printf '%s' "$content" | mlt_task_parse)"
total=$((checked + unchecked))

if [ "$status" = "done" ] && [ "$total" -gt 0 ] && [ "$checked" -lt "$total" ]; then
  emit \
    "" \
    "✗ task-gate: task $padded ($slug) — marked \"✅ done\" but $((total - checked))/$total acceptance criteria are still unchecked." \
    "" \
    "A task is \"done\" only when EVERY acceptance criterion is ticked and the shared Definition of Done holds. Either:" \
    "  • finish the work and tick the remaining criteria (- [ ] -> - [x]) in $taskfile, or" \
    "  • set **Status:** back to 🟡 partial until they are genuinely done." \
    "" \
    "Adhere to the acceptance criteria — don't claim completion you can't defend. Commit blocked."
  exit 1
fi

if [ "$total" -gt 0 ] && [ "$checked" -eq "$total" ] && [ "$status" != "done" ]; then
  emit \
    "" \
    "✗ task-gate: task $padded ($slug) — all $total acceptance criteria are checked but **Status:** is \"$status\", not done." \
    "" \
    "If the shared Definition of Done is also met, set **Status:** to ✅ done in $taskfile;" \
    "otherwise uncheck whatever isn't truly complete. Commit blocked."
  exit 1
fi

exit 0
