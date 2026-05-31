import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import type { ExtensionAPI, ExtensionCommandContext } from '@oh-my-pi/pi-coding-agent';

// A not-done task row as emitted by scripts/task-list.sh (tab-separated).
interface Task {
  num: string;
  slug: string;
  status: string;
  checked: string;
  total: string;
  title: string;
  path: string;
}

// Matches the "**Status:** ◻ not started" segment of a task doc's metadata line.
const NOT_STARTED = /(\*\*Status:\*\*\s*)◻\s*not started/;

function parseTaskRows(tsv: string): Task[] {
  const tasks: Task[] = [];
  for (const line of tsv.split('\n')) {
    if (!line.trim()) continue;
    const [num, slug, status, checked, total, title, path] = line.split('\t');
    if (!num || !slug || !path) continue;
    tasks.push({
      num,
      slug,
      status,
      checked: checked ?? '0',
      total: total ?? '0',
      title: title ?? '',
      path,
    });
  }
  return tasks;
}

// Bump a not-started task to "🟡 partial" in place. Returns true if the file actually changed.
function markPartial(absPath: string): boolean {
  const before = readFileSync(absPath, 'utf8');
  const after = before.replace(NOT_STARTED, '$1🟡 partial');
  if (after === before) return false;
  writeFileSync(absPath, after);
  return true;
}

async function startTask(pi: ExtensionAPI, ctx: ExtensionCommandContext): Promise<void> {
  if (!ctx.hasUI) {
    ctx.ui.notify('/mlt:start-task needs interactive mode', 'warning');
    return;
  }

  // Resolve the repo root so the command works from any cwd.
  const top = await pi.exec('git', ['rev-parse', '--show-toplevel'], { cwd: ctx.cwd });
  const root = top.code === 0 ? top.stdout.trim() : ctx.cwd;

  // Deterministic backlog read — zero LLM tokens.
  const listed = await pi.exec('bash', ['scripts/task-list.sh'], { cwd: root });
  if (listed.code !== 0) {
    ctx.ui.notify(`Couldn't read tasks: ${listed.stderr.trim() || 'task-list.sh failed'}`, 'error');
    return;
  }

  const tasks = parseTaskRows(listed.stdout).slice(0, 3);
  if (tasks.length === 0) {
    ctx.ui.notify('Every task is done — nothing to start.', 'info');
    return;
  }

  const labels = tasks.map((t) => `${t.title}  ·  ${t.status} (${t.checked}/${t.total})`);
  const picked = await ctx.ui.select('Start which task?', labels, {
    helpText: 'Pulls main, opens task/<n>-<slug>, marks it 🟡 partial · Esc to cancel',
  });
  if (!picked) return;
  const task = tasks[labels.indexOf(picked)];
  if (!task) return;

  // Refuse to operate on a dirty tree — never silently stash the user's work.
  const dirty = await pi.exec('git', ['status', '--porcelain'], { cwd: root });
  if (dirty.code !== 0) {
    ctx.ui.notify(`git status failed: ${dirty.stderr.trim()}`, 'error');
    return;
  }
  if (dirty.stdout.trim()) {
    ctx.ui.notify(
      'Working tree has uncommitted changes — commit or stash them before starting a task.',
      'warning',
    );
    return;
  }

  const branch = `task/${task.num}-${task.slug}`;

  // Pull latest main, then branch off it.
  const onMain = await pi.exec('git', ['checkout', 'main'], { cwd: root });
  if (onMain.code !== 0) {
    ctx.ui.notify(`Couldn't switch to main: ${onMain.stderr.trim()}`, 'error');
    return;
  }
  const pull = await pi.exec('git', ['pull', '--ff-only', 'origin', 'main'], { cwd: root });
  if (pull.code !== 0) {
    ctx.ui.notify(
      `Couldn't pull latest main (${pull.stderr.trim() || 'offline?'}) — branching from local main.`,
      'warning',
    );
  }

  // Create the task branch, or switch to it if it already exists.
  const exists = await pi.exec(
    'git',
    ['rev-parse', '--verify', '--quiet', `refs/heads/${branch}`],
    {
      cwd: root,
    },
  );
  const switched =
    exists.code === 0
      ? await pi.exec('git', ['checkout', branch], { cwd: root })
      : await pi.exec('git', ['checkout', '-b', branch], { cwd: root });
  if (switched.code !== 0) {
    ctx.ui.notify(`Couldn't create branch ${branch}: ${switched.stderr.trim()}`, 'error');
    return;
  }

  // Mark started. Left UNSTAGED on purpose: the pre-commit gate then forces the first commit
  // to carry the updated doc, keeping task progress honest with the code.
  let bumped = false;
  try {
    bumped = markPartial(join(root, task.path));
  } catch (err) {
    ctx.ui.notify(`Branch ready, but couldn't update ${task.path}: ${String(err)}`, 'warning');
  }

  ctx.ui.notify(
    [
      `Started ${task.num} — ${task.title}`,
      `• branch: ${branch}`,
      bumped
        ? `• ${task.path} marked 🟡 partial (unstaged)`
        : `• ${task.path} status unchanged (${task.status})`,
      '• tick acceptance criteria as you finish them — the pre-commit gate blocks until the doc reflects your work.',
    ].join('\n'),
    'success',
  );
}

export default function mltTasks(pi: ExtensionAPI): void {
  pi.setLabel('MLT Tasks');
  pi.registerCommand('mlt:start-task', {
    description: 'Pick a to-do task, pull main, branch, and mark it started',
    handler: (_args, ctx) => startTask(pi, ctx),
  });
}
