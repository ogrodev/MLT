<script lang="ts">
import { onMount } from 'svelte';
import {
  fetchClaudeUsage,
  onUsageError,
  onUsageUpdated,
  quitApp,
  type Status,
  type UsageSnapshot,
  type UsageWindow,
} from '$lib/usage';

let snapshot = $state<UsageSnapshot | null>(null);
let error = $state<string | null>(null);
let loading = $state(true);
let now = $state(Date.now());

const KIND_LABEL: Record<UsageWindow['kind'], string> = {
  Session: 'Session',
  Weekly: 'Weekly',
  Monthly: 'Monthly',
  Custom: 'Usage',
};

type Tone = 'ok' | 'warn' | 'err' | 'idle';

// Connected-state indicator. Always shows *something*: connecting before the first fetch,
// the provider's freshness once we have data, and a clear disconnected state on hard failure.
const TONE: Record<Tone, string> = {
  ok: 'text-emerald-600 dark:text-emerald-400',
  warn: 'text-amber-600 dark:text-amber-400',
  err: 'text-red-600 dark:text-red-400',
  idle: 'text-neutral-500 dark:text-neutral-400',
};
const STATUS_CONN: Record<Status, { label: string; tone: Tone }> = {
  Ok: { label: 'Connected', tone: 'ok' },
  Stale: { label: 'Stale', tone: 'warn' },
  Error: { label: 'Error', tone: 'err' },
};

const conn = $derived(
  snapshot
    ? STATUS_CONN[snapshot.status]
    : error
      ? { label: 'Disconnected', tone: 'err' as Tone }
      : { label: 'Connecting…', tone: 'idle' as Tone },
);

function label(w: UsageWindow): string {
  return w.reset_description ?? KIND_LABEL[w.kind];
}

function barColor(pct: number): string {
  if (pct >= 90) return 'bg-red-500';
  if (pct >= 70) return 'bg-amber-500';
  return 'bg-emerald-500';
}

function countdown(resetsAt: number | null): string {
  if (resetsAt == null) return '';
  const ms = resetsAt - now;
  if (ms <= 0) return 'resetting…';
  const mins = Math.floor(ms / 60000);
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const m = mins % 60;
  if (d > 0) return `resets in ${d}d ${h}h`;
  if (h > 0) return `resets in ${h}h ${m}m`;
  return `resets in ${m}m`;
}

function lastUpdated(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

onMount(() => {
  const unlisteners: Array<() => void> = [];

  fetchClaudeUsage()
    .then((s) => {
      snapshot = s;
      error = null;
    })
    .catch((e) => {
      error = String(e);
    })
    .finally(() => {
      loading = false;
    });

  onUsageUpdated((s) => {
    snapshot = s;
    error = null;
    loading = false;
  }).then((u) => unlisteners.push(u));
  onUsageError((msg) => {
    error = msg;
  }).then((u) => unlisteners.push(u));

  const tick = setInterval(() => {
    now = Date.now();
  }, 1000);

  return () => {
    clearInterval(tick);
    for (const u of unlisteners) u();
  };
});
</script>

<main
  class="flex h-screen w-screen flex-col bg-white font-sans text-neutral-900 select-none dark:bg-neutral-900 dark:text-neutral-100"
>
  <header
    class="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800"
  >
    <h1 class="text-sm font-semibold tracking-tight">Claude Code</h1>
    <span class="text-[11px] {TONE[conn.tone]}">● {conn.label}</span>
  </header>

  <section class="flex-1 overflow-y-auto px-4 py-3">
    {#if loading}
      <p class="mt-10 text-center text-sm text-neutral-500 dark:text-neutral-400">Loading usage…</p>
    {:else if error && !snapshot}
      <div class="mt-8 text-center">
        <p class="text-sm text-red-600 dark:text-red-400">Couldn't load usage</p>
        <p class="mt-2 text-[11px] break-words text-neutral-500 dark:text-neutral-400">{error}</p>
      </div>
    {:else if snapshot}
      <ul class="space-y-4">
        {#each snapshot.windows as w (w.kind + (w.reset_description ?? ''))}
          <li>
            <div class="mb-1 flex items-baseline justify-between">
              <span class="text-[13px] font-medium text-neutral-800 dark:text-neutral-200"
                >{label(w)}</span
              >
              <span class="text-[13px] text-neutral-600 tabular-nums dark:text-neutral-400"
                >{w.used_percent.toFixed(0)}%</span
              >
            </div>
            <div class="h-2 overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800">
              <div
                class="h-full rounded-full transition-[width] duration-500 {barColor(w.used_percent)}"
                style="width: {Math.min(100, Math.max(0, w.used_percent))}%"
              ></div>
            </div>
            {#if countdown(w.resets_at)}
              <p class="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                {countdown(w.resets_at)}
              </p>
            {/if}
          </li>
        {/each}
      </ul>
      {#if error}
        <p class="mt-4 text-center text-[11px] text-amber-600 dark:text-amber-400">stale · {error}</p>
      {/if}
    {/if}
  </section>

  <footer
    class="flex items-center justify-between border-t border-neutral-200 px-4 py-2 text-[11px] text-neutral-500 dark:border-neutral-800 dark:text-neutral-400"
  >
    <span>
      {#if snapshot}
        Updated {lastUpdated(snapshot.fetched_at)}
      {:else}
        MLT
      {/if}
    </span>
    <button
      type="button"
      onclick={() => quitApp()}
      class="rounded px-2 py-0.5 text-neutral-500 transition-colors hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
    >
      Quit
    </button>
  </footer>
</main>
