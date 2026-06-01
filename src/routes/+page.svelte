<script lang="ts">
import { onMount } from 'svelte';
import {
  fetchClaudeUsage,
  listSources,
  onUsageError,
  onUsageUpdated,
  quitApp,
  setSourceEnabled,
  type SourceState,
  type Status,
  type UsageSnapshot,
  type UsageWindow,
} from '$lib/usage';

let snapshot = $state<UsageSnapshot | null>(null);
let error = $state<string | null>(null);
let loading = $state(true);
let now = $state(Date.now());
let sources = $state<SourceState[]>([]);
let view = $state<'usage' | 'sources'>('usage');

const KIND_LABEL: Record<UsageWindow['kind'], string> = {
  Session: 'Session',
  Weekly: 'Weekly',
  Monthly: 'Monthly',
  Custom: 'Usage',
};

type Tone = 'ok' | 'warn' | 'err' | 'idle';

// Connected-state indicator. Always shows *something*: not-connected before any opt-in,
// connecting before the first fetch, the provider's freshness once we have data.
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

// A source is "connected" only when discovered *and* opted in (the consent gate).
const anyActive = $derived(sources.some((s) => s.present && s.enabled));

const conn = $derived(
  !anyActive
    ? { label: 'Not connected', tone: 'idle' as Tone }
    : snapshot
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

// Opt a source in/out. The backend persists the choice and, on opt-in, kicks an immediate
// fetch (a `usage-updated`/`usage-error` event), so the popover updates without a restart.
async function toggleSource(source: SourceState, enabled: boolean): Promise<void> {
  try {
    sources = await setSourceEnabled(source.id, enabled);
    error = null;
    if (enabled && source.present) {
      loading = true; // awaiting the backend's kick-off fetch
    } else if (!sources.some((s) => s.present && s.enabled)) {
      snapshot = null; // nothing connected anymore — drop the disconnected usage
      loading = false;
    }
  } catch (e) {
    error = String(e);
  }
}

onMount(() => {
  const unlisteners: Array<() => void> = [];

  listSources()
    .then((discovered) => {
      sources = discovered;
      // Only read a credential when a source is actually connected.
      if (discovered.some((s) => s.present && s.enabled)) {
        return fetchClaudeUsage().then((s) => {
          snapshot = s;
          error = null;
        });
      }
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
    loading = false;
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
    {#if view === 'sources'}
      <h1 class="text-sm font-semibold tracking-tight">Sources</h1>
      <button
        type="button"
        onclick={() => (view = 'usage')}
        class="rounded px-2 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
      >
        Done
      </button>
    {:else}
      <h1 class="text-sm font-semibold tracking-tight">Claude Code</h1>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={() => (view = 'sources')}
          class="rounded px-2 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
        >
          Sources
        </button>
        <span class="text-[11px] {TONE[conn.tone]}">● {conn.label}</span>
      </div>
    {/if}
  </header>

  <section class="flex-1 overflow-y-auto px-4 py-3">
    {#if view === 'sources'}
      <p class="mb-3 text-[11px] text-neutral-500 dark:text-neutral-400">
        MLT only reads a source after you turn it on. Discovery checks what's installed — it
        never reads a credential until you opt in.
      </p>
      <ul class="space-y-3">
        {#each sources as s (s.id)}
          <li class="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            <div class="flex items-start justify-between gap-3">
              <div class="flex items-center gap-2">
                <span class="text-[13px] font-medium text-neutral-800 dark:text-neutral-200"
                  >{s.display_name}</span
                >
                <span
                  class="rounded-full px-1.5 py-0.5 text-[10px] font-medium {s.present
                    ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300'
                    : 'bg-neutral-100 text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400'}"
                >
                  {s.present ? 'Detected' : 'Not detected'}
                </span>
              </div>
              <label
                class="relative inline-flex shrink-0 items-center {s.present
                  ? 'cursor-pointer'
                  : 'cursor-not-allowed opacity-40'}"
              >
                <input
                  type="checkbox"
                  class="peer sr-only"
                  checked={s.enabled}
                  disabled={!s.present}
                  onchange={(e) => toggleSource(s, e.currentTarget.checked)}
                />
                <span class="sr-only">Enable {s.display_name}</span>
                <span
                  class="block h-5 w-9 rounded-full bg-neutral-300 transition-colors peer-checked:bg-emerald-500 peer-focus-visible:ring-2 peer-focus-visible:ring-emerald-500/50 dark:bg-neutral-700"
                ></span>
                <span
                  class="pointer-events-none absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform peer-checked:translate-x-4"
                ></span>
              </label>
            </div>
            <p class="mt-2 text-[11px] leading-relaxed text-neutral-500 dark:text-neutral-400">
              {s.access_note}
            </p>
            {#if !s.present}
              <p class="mt-1 text-[11px] text-neutral-400 dark:text-neutral-500">
                Log in to {s.display_name} on this Mac, then it'll appear here.
              </p>
            {/if}
          </li>
        {/each}
      </ul>
    {:else if !anyActive}
      <div class="mt-8 text-center">
        <p class="text-sm text-neutral-700 dark:text-neutral-300">No sources connected</p>
        <p class="mt-2 text-[11px] text-neutral-500 dark:text-neutral-400">
          Choose what MLT may read to track your AI usage. Nothing is read until you opt in.
        </p>
        <button
          type="button"
          onclick={() => (view = 'sources')}
          class="mt-4 rounded-md bg-neutral-900 px-3 py-1.5 text-[12px] font-medium text-white transition-colors hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300"
        >
          Choose sources
        </button>
      </div>
    {:else if loading}
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
