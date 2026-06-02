<script lang="ts">
import { onMount } from 'svelte';
import {
  disconnectSource,
  fetchClaudeUsage,
  listSources,
  onUsageError,
  onUsageUpdated,
  quitApp,
  setApiKey,
  setSourceEnabled,
  setSourceLabel,
  type SourceState,
  type Status,
  type UsageSnapshot,
  type UsageWindow,
} from '$lib/usage';
import { getCurrentWindow } from '@tauri-apps/api/window';

let snapshot = $state<UsageSnapshot | null>(null);
let error = $state<string | null>(null);
let loading = $state(true);
let now = $state(Date.now());
let sources = $state<SourceState[]>([]);
let view = $state<'usage' | 'sources'>('usage');

// Ephemeral key-entry state for API-key sources (e.g. OpenRouter). `editingId` is the source
// whose key form is open; the draft is never persisted or echoed back once saved.
let editingId = $state<string | null>(null);
let draftKey = $state('');
let keyError = $state<string | null>(null);
let keyPending = $state(false);

// Which provider's usage the view is showing, plus the in-flight "set a name" form (one
// source at a time). `selectedId` is a preference — the shown provider is derived with a
// fallback so the view stays coherent as connections change.
let selectedId = $state<string | null>(null);
let editingNameId = $state<string | null>(null);
let nameDraft = $state('');
let nameError = $state<string | null>(null);

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
// A local-login source is connected when discovered *and* opted in; an API-key source is
// connected when a (validated) key is stored — i.e. simply enabled, nothing to detect.
function sourceActive(s: SourceState): boolean {
  return s.credential === 'ApiKey' ? s.enabled : s.present && s.enabled;
}

// Connected providers and the one currently shown. The shown provider falls back to Claude,
// then the first connected source, so a stale or empty `selectedId` never blanks the view.
const activeSources = $derived(sources.filter(sourceActive));
const selected = $derived.by(() => {
  if (activeSources.length === 0) return null;
  const pick = selectedId ? activeSources.find((s) => s.id === selectedId) : undefined;
  return pick ?? activeSources.find((s) => s.id === 'claude-code') ?? activeSources[0];
});

// The account identifier (email, else org) for the *selected* provider, shown as a subtitle.
// Prefer the live snapshot's identity, but only for the same provider — never render one
// provider's identity under another — then fall back to the cached identity on the row.
const selectedEmail = $derived.by((): string | null => {
  if (!selected) return null;
  if (snapshot && snapshot.provider === selected.id && snapshot.account) {
    return snapshot.account.email ?? snapshot.account.organization;
  }
  return selected.account?.email ?? selected.account?.organization ?? null;
});

const conn = $derived.by((): { label: string; tone: Tone } => {
  if (!selected) return { label: 'Not connected', tone: 'idle' };
  // Only Claude reports usage today; any other connected source is simply "Connected".
  if (selected.id !== 'claude-code') return { label: 'Connected', tone: 'ok' };
  if (snapshot) return STATUS_CONN[snapshot.status];
  if (error) return { label: 'Disconnected', tone: 'err' };
  return { label: 'Connecting…', tone: 'idle' };
});

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
    if (enabled && source.present && !snapshot) {
      // Awaiting the backend's kick-off fetch — unless a usage event already raced in and
      // populated the snapshot, in which case flipping to "loading" would mask live data.
      loading = true;
    } else if (!sources.some((s) => s.id === 'claude-code' && sourceActive(s))) {
      snapshot = null; // nothing connected anymore — drop the disconnected usage
      loading = false;
    }
  } catch (e) {
    error = String(e);
  }
}

function startEditKey(id: string): void {
  editingId = id;
  draftKey = '';
  keyError = null;
}

function cancelEditKey(): void {
  editingId = null;
  draftKey = '';
  keyError = null;
}

// Enter/replace a key. The backend validates it against the provider before storing, so a
// rejected key throws here with a clear message and the source stays disconnected.
async function saveKey(source: SourceState): Promise<void> {
  if (!draftKey.trim()) return;
  keyPending = true;
  keyError = null;
  try {
    sources = await setApiKey(source.id, draftKey);
    cancelEditKey();
  } catch (e) {
    keyError = String(e);
  } finally {
    keyPending = false;
  }
}

async function disconnectKeySource(source: SourceState): Promise<void> {
  keyError = null;
  try {
    sources = await disconnectSource(source.id);
  } catch (e) {
    keyError = String(e);
  }
}

function startEditName(s: SourceState): void {
  editingNameId = s.id;
  nameDraft = s.label ?? '';
  nameError = null;
}

function cancelEditName(): void {
  editingNameId = null;
  nameDraft = '';
  nameError = null;
}

// Persist a source's display name (an empty value clears it). Returns the refreshed list.
async function saveName(source: SourceState): Promise<void> {
  nameError = null;
  try {
    sources = await setSourceLabel(source.id, nameDraft);
    cancelEditName();
  } catch (e) {
    nameError = String(e);
  }
}

onMount(() => {
  const unlisteners: Array<() => void> = [];

  listSources()
    .then((discovered) => {
      sources = discovered;
      // Only read a credential when a source is actually connected.
      if (discovered.some((s) => s.id === 'claude-code' && sourceActive(s))) {
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

  // Re-discover whenever the popover regains focus (i.e. each time it's opened), so presence
  // reflects logins/logouts that happened since the webview loaded — `sources` is otherwise
  // only fetched once. Passive refresh: swallow errors so it can't clobber the usage state.
  getCurrentWindow()
    .onFocusChanged(({ payload: focused }) => {
      if (focused) {
        listSources()
          .then((s) => {
            sources = s;
          })
          .catch(() => {});
      }
    })
    .then((u) => unlisteners.push(u));

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
  {#snippet providerIcon(id: string)}
    {#if id === 'claude-code'}
      <svg viewBox="0 0 24 24" fill="currentColor" class="h-4 w-4 shrink-0" aria-hidden="true">
        <path
          d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"
        />
      </svg>
    {:else if id === 'openrouter'}
      <svg viewBox="0 0 24 24" fill="currentColor" class="h-4 w-4 shrink-0" aria-hidden="true">
        <path
          d="M16.778 1.844v1.919q-.569-.026-1.138-.032-.708-.008-1.415.037c-1.93.126-4.023.728-6.149 2.237-2.911 2.066-2.731 1.95-4.14 2.75-.396.223-1.342.574-2.185.798-.841.225-1.753.333-1.751.333v4.229s.768.108 1.61.333c.842.224 1.789.575 2.185.799 1.41.798 1.228.683 4.14 2.75 2.126 1.509 4.22 2.11 6.148 2.236.88.058 1.716.041 2.555.005v1.918l7.222-4.168-7.222-4.17v2.176c-.86.038-1.611.065-2.278.021-1.364-.09-2.417-.357-3.979-1.465-2.244-1.593-2.866-2.027-3.68-2.508.889-.518 1.449-.906 3.822-2.59 1.56-1.109 2.614-1.377 3.978-1.466.667-.044 1.418-.017 2.278.02v2.176L24 6.014Z"
        />
      </svg>
    {:else}
      <svg viewBox="0 0 24 24" fill="currentColor" class="h-4 w-4 shrink-0" aria-hidden="true">
        <circle cx="12" cy="12" r="5" />
      </svg>
    {/if}
  {/snippet}

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
      <h1 class="min-w-0 truncate text-sm font-semibold tracking-tight">
        {selected ? selected.display_name : 'MLT'}
      </h1>
      <div class="flex shrink-0 items-center gap-2">
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
          {@const canToggle = s.present || s.enabled}
          <li class="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            {#if s.credential === 'ApiKey'}
              <div class="flex items-start justify-between gap-3">
                <span class="text-[13px] font-medium text-neutral-800 dark:text-neutral-200"
                  >{s.display_name}</span
                >
                <span
                  class="rounded-full px-1.5 py-0.5 text-[10px] font-medium {s.enabled
                    ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300'
                    : 'bg-neutral-100 text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400'}"
                >
                  {s.enabled ? 'Connected' : 'Not connected'}
                </span>
              </div>
              <p class="mt-2 text-[11px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                {s.access_note}
              </p>
              {#if editingId === s.id}
                <form
                  class="mt-3"
                  onsubmit={(e) => {
                    e.preventDefault();
                    saveKey(s);
                  }}
                >
                  <input
                    type="password"
                    bind:value={draftKey}
                    placeholder="Paste API key (e.g. sk-or-v1…)"
                    aria-label="API key for {s.display_name}"
                    autocomplete="off"
                    autocapitalize="off"
                    spellcheck="false"
                    disabled={keyPending}
                    class="w-full rounded-md border border-neutral-300 bg-white px-2 py-1 text-[12px] text-neutral-900 placeholder:text-neutral-400 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
                  />
                  {#if keyError}
                    <p class="mt-1 text-[11px] break-words text-red-600 dark:text-red-400">
                      {keyError}
                    </p>
                  {/if}
                  <div class="mt-2 flex items-center gap-2">
                    <button
                      type="submit"
                      disabled={keyPending || !draftKey.trim()}
                      class="rounded-md bg-neutral-900 px-3 py-1 text-[12px] font-medium text-white transition-colors hover:bg-neutral-700 disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300"
                    >
                      {keyPending ? 'Verifying…' : 'Save key'}
                    </button>
                    <button
                      type="button"
                      onclick={cancelEditKey}
                      disabled={keyPending}
                      class="rounded px-2 py-1 text-[12px] text-neutral-500 transition-colors hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                    >
                      Cancel
                    </button>
                  </div>
                </form>
              {:else}
                <div class="mt-3 flex items-center gap-2">
                  <button
                    type="button"
                    onclick={() => startEditKey(s.id)}
                    class="rounded-md border border-neutral-300 px-2.5 py-1 text-[12px] font-medium text-neutral-700 transition-colors hover:bg-neutral-100 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
                  >
                    {s.enabled ? 'Replace key' : 'Add key'}
                  </button>
                  {#if s.enabled}
                    <button
                      type="button"
                      onclick={() => disconnectKeySource(s)}
                      class="rounded px-2 py-1 text-[12px] text-red-600 transition-colors hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-950/40"
                    >
                      Disconnect
                    </button>
                  {/if}
                </div>
                {#if keyError}
                  <p class="mt-1 text-[11px] break-words text-red-600 dark:text-red-400">
                    {keyError}
                  </p>
                {/if}
              {/if}
            {:else}
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
                  class="relative inline-flex shrink-0 items-center {canToggle
                    ? 'cursor-pointer'
                    : 'cursor-not-allowed opacity-40'}"
                >
                  <input
                    type="checkbox"
                    class="peer sr-only"
                    checked={s.enabled}
                    disabled={!canToggle}
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
              {#if !s.present && !s.enabled}
                <p class="mt-1 text-[11px] text-neutral-400 dark:text-neutral-500">
                  Log in to {s.display_name} on this Mac, then it'll appear here.
                </p>
              {:else if !s.present}
                <p class="mt-1 text-[11px] text-neutral-400 dark:text-neutral-500">
                  Not detected right now — turn off to revoke; it resumes if detected again.
                </p>
              {/if}
            {/if}
            <div class="mt-3 border-t border-neutral-100 pt-3 dark:border-neutral-800">
              {#if s.account?.email}
                <p class="mb-2 truncate text-[11px] text-neutral-500 dark:text-neutral-400">
                  Account
                  <span class="font-medium text-neutral-700 dark:text-neutral-300"
                    >{s.account.email}</span
                  >
                </p>
              {/if}
              {#if editingNameId === s.id}
                <form
                  class="flex items-center gap-2"
                  onsubmit={(e) => {
                    e.preventDefault();
                    saveName(s);
                  }}
                >
                  <input
                    type="text"
                    bind:value={nameDraft}
                    placeholder="Custom name"
                    aria-label="Custom name for {s.display_name}"
                    autocomplete="off"
                    class="min-w-0 flex-1 rounded-md border border-neutral-300 bg-white px-2 py-1 text-[12px] text-neutral-900 placeholder:text-neutral-400 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
                  />
                  <button
                    type="submit"
                    class="shrink-0 rounded-md bg-neutral-900 px-2.5 py-1 text-[12px] font-medium text-white transition-colors hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300"
                  >
                    Save
                  </button>
                  <button
                    type="button"
                    onclick={cancelEditName}
                    class="shrink-0 rounded px-2 py-1 text-[12px] text-neutral-500 transition-colors hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                  >
                    Cancel
                  </button>
                </form>
                {#if nameError}
                  <p class="mt-1 text-[11px] break-words text-red-600 dark:text-red-400">
                    {nameError}
                  </p>
                {/if}
              {:else}
                <div class="flex items-center justify-between gap-2">
                  <span class="min-w-0 truncate text-[11px] text-neutral-500 dark:text-neutral-400">
                    {#if s.label}
                      Shown as <span
                        class="font-medium text-neutral-700 dark:text-neutral-300">{s.label}</span
                      >
                    {:else}
                      Using default name
                    {/if}
                  </span>
                  <button
                    type="button"
                    onclick={() => startEditName(s)}
                    class="shrink-0 rounded px-2 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                  >
                    {s.label ? 'Rename' : 'Add name'}
                  </button>
                </div>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {:else if activeSources.length === 0}
      <div class="mt-8 text-center">
        <p class="text-sm text-neutral-700 dark:text-neutral-300">No usage to show yet</p>
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
    {:else}
      {#if activeSources.length > 1}
        <div class="mb-4 flex gap-1 rounded-lg bg-neutral-100 p-1 dark:bg-neutral-800">
          {#each activeSources as p (p.id)}
            <button
              type="button"
              onclick={() => (selectedId = p.id)}
              title={p.display_name}
              class="flex min-w-0 flex-1 flex-col items-center gap-1 rounded-md px-2 py-1.5 text-[11px] font-medium transition-colors {selected?.id ===
              p.id
                ? 'bg-white text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-100'
                : 'text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-200'}"
            >
              {@render providerIcon(p.id)}
              <span class="max-w-full truncate">{p.display_name}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if selected && (selected.label || selectedEmail)}
        <div class="mb-4">
          {#if selected.label}
            <h2 class="truncate text-[15px] font-semibold tracking-tight">{selected.label}</h2>
          {/if}
          {#if selectedEmail}
            <p class="truncate text-[11px] text-neutral-500 dark:text-neutral-400">
              {selectedEmail}
            </p>
          {/if}
        </div>
      {/if}

      {#if selected && selected.id !== 'claude-code'}
        <div class="mt-8 text-center">
          <p class="text-sm text-neutral-700 dark:text-neutral-300">
            {selected.display_name} is connected
          </p>
          <p class="mt-2 text-[11px] text-neutral-500 dark:text-neutral-400">
            Usage tracking for this provider is coming in a later update.
          </p>
        </div>
      {:else if loading}
        <p class="mt-10 text-center text-sm text-neutral-500 dark:text-neutral-400">
          Loading usage…
        </p>
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
          <p class="mt-4 text-center text-[11px] text-amber-600 dark:text-amber-400">
            stale · {error}
          </p>
        {/if}
      {/if}
    {/if}
  </section>

  <footer
    class="flex items-center justify-between border-t border-neutral-200 px-4 py-2 text-[11px] text-neutral-500 dark:border-neutral-800 dark:text-neutral-400"
  >
    <span>
      {#if selected?.id === 'claude-code' && snapshot}
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
