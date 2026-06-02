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

// Read a task doc for inlining into the start prompt. Returns null when it can't be read, so the
// caller can fall back to a path-only nudge instead of failing the whole command.
function readTaskSpec(absPath: string): string | null {
  try {
    return readFileSync(absPath, 'utf8').trimEnd();
  } catch {
    return null;
  }
}

// A fence at least one backtick longer than the longest backtick run in `body`, so inlined
// content (a task doc often contains its own ``` fences) can never close the wrapper early.
function fenceFor(body: string): string {
  const longest = body.match(/`+/g)?.reduce((max, run) => Math.max(max, run.length), 0) ?? 0;
  return '`'.repeat(Math.max(3, longest + 1));
}

// Build the user prompt that hands the freshly-branched task to the agent. Inlines the task doc
// (the acceptance criteria + Definition of Done the pre-commit gate enforces) so the agent starts
// without hunting for it.
function startPrompt(task: Task, branch: string, spec: string | null): string {
  const lines = [
    `Start work on task ${task.title}.`,
    '',
    `Branch \`${branch}\` is checked out; \`${task.path}\` is marked 🟡 partial (unstaged).`,
    '',
  ];
  if (spec) {
    const fence = fenceFor(spec);
    lines.push(`Task spec — \`${task.path}\`:`, '', `${fence}markdown`, spec, fence, '');
  } else {
    lines.push(`Read the task spec at \`${task.path}\` first (it could not be inlined here).`, '');
  }
  lines.push(
    'Read every doc and ADR the task references, plan the work, then implement it end-to-end.',
    `As you satisfy each acceptance criterion, tick it (\`- [ ]\` -> \`- [x]\`) in \`${task.path}\`; set **Status:** to ✅ done only once every criterion and the shared Definition of Done hold. The pre-commit task-gate blocks your first commit until that doc reflects real progress.`,
    'Run `make check` before you finish.',
  );
  return lines.join('\n');
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

  // Hand the task straight to the agent instead of stopping at a toast: inline the spec and inject
  // it as a user prompt. When idle this starts a turn immediately; if a turn is somehow already
  // running, queue it to fire right after.
  const spec = readTaskSpec(join(root, task.path));
  const idle = ctx.isIdle();

  ctx.ui.notify(
    [
      `Started ${task.title}`,
      `• branch: ${branch}`,
      bumped
        ? `• ${task.path} marked 🟡 partial (unstaged)`
        : `• ${task.path} status unchanged (${task.status})`,
      idle ? '• handing it to the agent now...' : '• queued for the agent — runs after this turn.',
    ].join('\n'),
    'success',
  );

  // sendUserMessage goes through the prompt flow, which can reject (e.g. model/API-key
  // validation when idle). Handle it instead of leaving an unhandled rejection after we already
  // toasted success — the branch itself is already prepared regardless.
  pi.sendUserMessage(
    startPrompt(task, branch, spec),
    idle ? undefined : { deliverAs: 'followUp' },
  ).catch((err) => {
    ctx.ui.notify(
      `Task branch is ready, but handing it to the agent failed: ${String(err)}`,
      'error',
    );
  });
}

export default function mltTasks(pi: ExtensionAPI): void {
  pi.setLabel('MLT Tasks');
  pi.registerCommand('mlt:start-task', {
    description: 'Pick a to-do task, pull main, branch, and mark it started',
    handler: (_args, ctx) => startTask(pi, ctx),
  });
}
