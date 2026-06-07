<script lang="ts">
  /// Agent dashboard (ai-app.md §2.2) — the pull / observability
  /// surface. A4: the read-only activity timeline (trigger → gate → act
  /// → audit, newest first), read from the tamper-evident audit ledger
  /// via the shared S-U4 read command. Behaviour status (A6), per-item
  /// Undo (A5), and anomaly notices stay as honest placeholders.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Activity, History, Bell, RefreshCw, ShieldAlert, Eye, Sparkles, PowerOff } from "@lucide/svelte";
  import AgentFilters from "$lib/components/AgentFilters.svelte";

  interface ActivityEntry {
    index: number;
    timestampMicros: number;
    kind: string;
    actor: string;
    subject: string;
    outcome: string;
    nodeTypes: string[];
    relations: string[];
    resultCount: number | null;
    durationMs: number | null;
    depth: number | null;
    callChainId: string | null;
    projectId: string | null;
    entryRef: string;
  }
  interface ActivityPage {
    entries: ActivityEntry[];
    available: boolean;
    tampered: boolean;
    total: number;
  }
  interface BehaviourStatus {
    name: string;
    description: string;
    kind: string;
    provenance: string;
    enabled: boolean;
    disabledReason: string | null;
    reads: string;
  }
  interface BehaviourReport {
    behaviours: BehaviourStatus[];
    errors: string[];
  }
  interface Notice {
    kind: string;
    summary: string;
    body: string;
    critical: boolean;
    tsMicros: number;
  }
  interface Capability {
    enabled: boolean;
    tier: string;
    actionMode: string;
    provider?: string | null;
    model?: string | null;
    executorLive: boolean;
  }

  /// Human label + semantic tone per audit kind.
  const KIND_META: Record<string, { label: string; tone: string }> = {
    query: { label: "Query", tone: "neutral" },
    "tool-call": { label: "Tool call", tone: "info" },
    confirm: { label: "Confirmed", tone: "ok" },
    "policy-violation": { label: "Blocked", tone: "warn" },
    "graph-access": { label: "Graph", tone: "neutral" },
    permission: { label: "Permission", tone: "info" },
    "network-call": { label: "Network", tone: "info" },
  };

  let activity = $state<ActivityPage | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let behaviours = $state<BehaviourReport | null>(null);
  let notices = $state<Notice[] | null>(null);
  let capability = $state<Capability | null>(null);

  // Activity-timeline filters (A7 inc 3). User-driven `$state` (not an IPC
  // callback), so plain reactivity is reliable here.
  let selectedKind = $state<string | null>(null);
  let selectedOutcome = $state<string | null>(null);
  let timeWindow = $state("all");

  // Filter options are derived from the loaded entries, not hardcoded, so only
  // what the ledger actually contains is offered.
  const kindOptions = $derived(
    [...new Set((activity?.entries ?? []).map((e) => e.kind))]
      .sort()
      .map((k) => ({ value: k, label: KIND_META[k]?.label ?? k })),
  );
  const outcomeOptions = $derived(
    [...new Set((activity?.entries ?? []).map((e) => e.outcome))]
      .sort()
      .map((o) => ({ value: o, label: o })),
  );

  const WINDOW_MS: Record<string, number | null> = {
    all: null,
    "1h": 3_600_000,
    "24h": 86_400_000,
    "7d": 604_800_000,
  };

  // The visible timeline after applying the filters. Time compares against the
  // wall clock; entry timestamps are microseconds, the window is milliseconds.
  const filteredEntries = $derived.by(() => {
    const entries = activity?.entries ?? [];
    const windowMs = WINDOW_MS[timeWindow] ?? null;
    const now = Date.now();
    return entries.filter(
      (e) =>
        (selectedKind === null || e.kind === selectedKind) &&
        (selectedOutcome === null || e.outcome === selectedOutcome) &&
        (windowMs === null || now - e.timestampMicros / 1000 <= windowMs),
    );
  });

  function relativeTime(micros: number): string {
    const then = micros / 1000;
    const diffSec = Math.max(0, (Date.now() - then) / 1000);
    if (diffSec < 45) return "just now";
    if (diffSec < 90) return "a minute ago";
    const min = Math.round(diffSec / 60);
    if (min < 60) return `${min} min ago`;
    const hr = Math.round(min / 60);
    if (hr < 24) return `${hr} h ago`;
    const day = Math.round(hr / 24);
    if (day < 7) return `${day} d ago`;
    return new Date(then).toLocaleDateString();
  }

  async function load() {
    loading = true;
    error = null;
    // Behaviour status is independent of the audit ledger: load it
    // best-effort so an audit-daemon outage does not blank the behaviour
    // list, and vice versa.
    try {
      behaviours = await invoke<BehaviourReport>("ai_behaviours");
    } catch {
      behaviours = null;
    }
    try {
      notices = await invoke<Notice[]>("ai_notices");
    } catch {
      notices = null;
    }
    // The acting posture (executor_live) is config, not ledger state, so
    // it loads independently and a daemon outage never blanks it.
    try {
      capability = await invoke<Capability>("ai_capability");
    } catch {
      capability = null;
    }
    try {
      activity = await invoke<ActivityPage>("ai_activity_recent", { limit: 100 });
    } catch (e) {
      error = String(e);
      activity = null;
    } finally {
      loading = false;
    }
  }

  onMount(load);
</script>

<div class="agent-shell">
  <AgentFilters
    kinds={kindOptions}
    outcomes={outcomeOptions}
    bind:selectedKind
    bind:selectedOutcome
    bind:timeWindow
  />
  <div class="agent-main">
<Page
  title="Agent"
  description="What the assistant has done on your behalf. Read-only, from the tamper-evident audit ledger — review each curated action and undo it if you want."
>
  {#if capability}
    {#if !capability.enabled}
      <div class="posture" data-mode="off">
        <PowerOff size={16} strokeWidth={1.75} />
        <div>
          <span class="posture-title">AI layer disabled</span>
          <span class="posture-sub">The agent does nothing until it is enabled in Settings → AI.</span>
        </div>
      </div>
    {:else if capability.executorLive}
      <div class="posture" data-mode="act">
        <Sparkles size={16} strokeWidth={1.75} />
        <div>
          <span class="posture-title">Acting</span>
          <span class="posture-sub">The agent writes safe, reversible curation automatically. Review each action below and undo it if you want.</span>
        </div>
      </div>
    {:else}
      <div class="posture" data-mode="suggest">
        <Eye size={16} strokeWidth={1.75} />
        <div>
          <span class="posture-title">Suggest-only</span>
          <span class="posture-sub">The agent computes and proposes curation but writes nothing yet. The activity below is what it observed; turn on the executor in Settings → AI to let it act.</span>
        </div>
      </div>
    {/if}
  {/if}

  <SectionGrid>
    <Group label="Activity" class="span-full">
      <div class="activity-head">
        <div class="activity-head-text">
          <History size={15} strokeWidth={1.75} />
          <span>
            Trigger → gate → act → audit, newest first.{#if activity && activity.total > 0}
              <span class="activity-count">{activity.total} total</span>{/if}
          </span>
        </div>
        <Button variant="ghost" size="sm" disabled={loading} onclick={load}>
          <RefreshCw size={14} class={loading ? "spin" : ""} />
          Refresh
        </Button>
      </div>

      {#if activity?.tampered}
        <div class="banner">
          <ShieldAlert size={16} />
          The audit ledger reports tampering. The entries below may be incomplete.
        </div>
      {/if}

      {#if error}
        <p class="empty">Activity unavailable: {error}</p>
      {:else if !activity || !activity.available}
        <p class="empty">The audit daemon is not running, so there is no activity to show yet.</p>
      {:else if activity.entries.length === 0}
        <p class="empty">No AI activity recorded yet.</p>
      {:else if filteredEntries.length === 0}
        <p class="empty">No activity matches the current filters.</p>
      {:else}
        <ul class="timeline">
          {#each filteredEntries as entry (entry.entryRef)}
            {@const meta = KIND_META[entry.kind] ?? { label: entry.kind, tone: "neutral" }}
            <li class="item">
              <span class="badge" data-tone={meta.tone}>{meta.label}</span>
              <div class="body">
                <div class="line">
                  <span class="subject">{entry.subject}</span>
                  <span class="outcome" data-outcome={entry.outcome}>{entry.outcome}</span>
                </div>
                <div class="detail">
                  <span>{entry.actor}</span>
                  {#if entry.relations.length > 0}
                    <span class="sep">·</span><span>{entry.relations.join(", ")}</span>
                  {/if}
                  {#if entry.resultCount !== null}
                    <span class="sep">·</span><span>{entry.resultCount} result{entry.resultCount === 1 ? "" : "s"}</span>
                  {/if}
                  {#if entry.durationMs !== null}
                    <span class="sep">·</span><span>{entry.durationMs} ms</span>
                  {/if}
                </div>
              </div>
              <time class="time">{relativeTime(entry.timestampMicros)}</time>
            </li>
          {/each}
        </ul>
      {/if}
    </Group>

    <Group label="Behaviours">
      {#if !behaviours}
        <p class="empty">Behaviour status unavailable.</p>
      {:else if behaviours.behaviours.length === 0 && behaviours.errors.length === 0}
        <p class="empty">No agent behaviours are installed.</p>
      {:else}
        <p class="bh-hint">
          <Activity size={14} strokeWidth={1.75} />
          The set the agent would act on. Enabling and disabling stays in Settings → AI.
        </p>
        <p class="bh-legend">
          <span><span class="bh-kind">workflow</span> runs deterministically with no LLM call.</span>
          <span><span class="bh-kind">agent</span> runs a bounded LLM loop.</span>
        </p>
        {#if behaviours.errors.length > 0}
          <div class="banner">
            <ShieldAlert size={16} />
            <div>
              {behaviours.errors.length} behaviour director{behaviours.errors.length === 1 ? "y" : "ies"} failed to load:
              <ul class="bh-errors">
                {#each behaviours.errors as err}<li>{err}</li>{/each}
              </ul>
            </div>
          </div>
        {/if}
        <ul class="bh-list">
          {#each behaviours.behaviours as b (b.name)}
            <li class="bh-item">
              <span class="badge" data-tone={b.enabled ? "ok" : "neutral"}>
                {b.enabled ? "enabled" : "disabled"}
              </span>
              <div class="body">
                <div class="line">
                  <span class="subject">{b.name}</span>
                  <span class="bh-kind">{b.kind}</span>
                  <span class="bh-prov">{b.provenance}</span>
                </div>
                <div class="detail">
                  {#if b.description}<span>{b.description}</span>{/if}
                  {#if !b.enabled && b.disabledReason}
                    <span class="sep">·</span><span class="outcome" data-outcome="denied">{b.disabledReason}</span>
                  {/if}
                </div>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </Group>

    <Group label="Notices">
      {#if !notices || notices.length === 0}
        <div class="placeholder">
          <Bell size={20} strokeWidth={1.5} />
          <p>Rare, important warnings from the Anomaly Detector surface here.
            The agent itself never pushes. Nothing to show right now.</p>
        </div>
      {:else}
        <ul class="bh-list">
          {#each notices as n (n.tsMicros + n.summary)}
            <li class="item">
              <span class="badge" data-tone={n.critical ? "warn" : "info"}>
                {n.critical ? "critical" : "notice"}
              </span>
              <div class="body">
                <div class="line">
                  <span class="subject">{n.summary}</span>
                </div>
                <div class="detail">
                  {#if n.body}<span>{n.body}</span>{/if}
                </div>
              </div>
              <time class="time">{relativeTime(n.tsMicros)}</time>
            </li>
          {/each}
        </ul>
      {/if}
    </Group>
  </SectionGrid>
</Page>
  </div>
</div>

<style>
  .agent-shell {
    display: flex;
    min-height: 100%;
  }
  .agent-main {
    flex: 1;
    min-width: 0;
  }
  .activity-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.25rem;
  }
  .activity-head-text {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    line-height: 1.4;
  }
  .activity-head-text :global(svg) {
    flex-shrink: 0;
    margin-top: 0.1rem;
  }
  .activity-count {
    margin-left: 0.375rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 0.75rem;
    margin-bottom: 0.75rem;
    border-radius: 0.5rem;
    font-size: 0.8125rem;
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-error) 30%, transparent);
  }
  .empty {
    margin: 0;
    padding: 1rem 0;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .timeline {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .item {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.625rem 0;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .item:first-child {
    border-top: none;
  }
  .badge {
    flex-shrink: 0;
    min-width: 5.5rem;
    text-align: center;
    padding: 0.125rem 0.5rem;
    border-radius: 0.375rem;
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .badge[data-tone="ok"] {
    color: #16a34a;
    background: color-mix(in srgb, #16a34a 14%, transparent);
  }
  .badge[data-tone="warn"] {
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 14%, transparent);
  }
  .badge[data-tone="info"] {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  }
  .body {
    flex: 1;
    min-width: 0;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }
  .subject {
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .outcome {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .outcome[data-outcome="denied"],
  .outcome[data-outcome="error"] {
    color: var(--color-error);
  }
  .detail {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.125rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .sep {
    opacity: 0.5;
  }
  .time {
    flex-shrink: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
  .placeholder {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.85rem;
    line-height: 1.5;
  }
  .placeholder p {
    margin: 0;
  }
  .bh-hint {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 0.5rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .bh-hint :global(svg) {
    flex-shrink: 0;
  }
  .bh-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .bh-item {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.625rem 0;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .bh-item:first-child {
    border-top: none;
  }
  .bh-kind,
  .bh-prov {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .bh-errors {
    margin: 0.25rem 0 0;
    padding-left: 1rem;
    font-size: 0.75rem;
  }
  :global(.spin) {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .posture {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    padding: 0.7rem 0.85rem;
    margin-bottom: 1rem;
    border-radius: 0.6rem;
    border: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .posture :global(svg) {
    flex-shrink: 0;
    margin-top: 0.1rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .posture[data-mode="act"] {
    border-color: color-mix(in srgb, var(--color-accent) 35%, transparent);
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
  }
  .posture[data-mode="act"] :global(svg) {
    color: var(--color-accent);
  }
  .posture > div {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .posture-title {
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .posture-sub {
    font-size: 0.78rem;
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .bh-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 1rem;
    margin: -0.25rem 0 0.6rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .bh-legend .bh-kind {
    font-family: var(--font-mono, monospace);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
</style>
