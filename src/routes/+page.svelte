<script lang="ts">
import { onMount } from 'svelte';
import {
  fetchClaudeUsage,
  onUsageError,
  onUsageUpdated,
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

<main class="flex h-screen w-screen flex-col bg-neutral-900 font-sans text-neutral-100 select-none">
  <header class="flex items-center justify-between border-b border-neutral-800 px-4 py-3">
    <h1 class="text-sm font-semibold tracking-tight">Claude Code</h1>
    {#if snapshot}
      <span
        class="text-[11px] {snapshot.status === 'Ok'
          ? 'text-emerald-400'
          : snapshot.status === 'Stale'
            ? 'text-amber-400'
            : 'text-red-400'}"
      >
        ● {snapshot.status}
      </span>
    {/if}
  </header>

  <section class="flex-1 overflow-y-auto px-4 py-3">
    {#if loading}
      <p class="mt-10 text-center text-sm text-neutral-500">Loading usage…</p>
    {:else if error && !snapshot}
      <div class="mt-8 text-center">
        <p class="text-sm text-red-400">Couldn't load usage</p>
        <p class="mt-2 text-[11px] break-words text-neutral-500">{error}</p>
      </div>
    {:else if snapshot}
      <ul class="space-y-4">
        {#each snapshot.windows as w (w.kind + (w.reset_description ?? ''))}
          <li>
            <div class="mb-1 flex items-baseline justify-between">
              <span class="text-[13px] font-medium text-neutral-200">{label(w)}</span>
              <span class="text-[13px] text-neutral-400 tabular-nums">{w.used_percent.toFixed(0)}%</span>
            </div>
            <div class="h-2 overflow-hidden rounded-full bg-neutral-800">
              <div
                class="h-full rounded-full transition-[width] duration-500 {barColor(w.used_percent)}"
                style="width: {Math.min(100, Math.max(0, w.used_percent))}%"
              ></div>
            </div>
            {#if countdown(w.resets_at)}
              <p class="mt-1 text-[11px] text-neutral-500">{countdown(w.resets_at)}</p>
            {/if}
          </li>
        {/each}
      </ul>
      {#if error}
        <p class="mt-4 text-center text-[11px] text-amber-400">stale · {error}</p>
      {/if}
    {/if}
  </section>

  <footer class="border-t border-neutral-800 px-4 py-2 text-[11px] text-neutral-500">
    {#if snapshot}
      Updated {lastUpdated(snapshot.fetched_at)}
    {:else}
      MLT
    {/if}
  </footer>
</main>
