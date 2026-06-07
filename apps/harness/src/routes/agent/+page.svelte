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
  import { Activity, History, Bell, RefreshCw, ShieldAlert } from "@lucide/svelte";

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
  let behaviours = $state<BehaviourStatus[] | null>(null);

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
      behaviours = await invoke<BehaviourStatus[]>("ai_behaviours");
    } catch {
      behaviours = null;
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

<Page
  title="Agent"
  description="What the assistant has done on your behalf. Read-only, from the tamper-evident audit ledger — review each curated action and undo it if you want."
>
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
      {:else}
        <ul class="timeline">
          {#each activity.entries as entry (entry.entryRef)}
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
      {:else if behaviours.length === 0}
        <p class="empty">No agent behaviours are installed.</p>
      {:else}
        <p class="bh-hint">
          <Activity size={14} strokeWidth={1.75} />
          The set the agent would act on. Enabling and disabling stays in Settings → AI.
        </p>
        <ul class="bh-list">
          {#each behaviours as b (b.name)}
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
      <div class="placeholder">
        <Bell size={20} strokeWidth={1.5} />
        <p>Rare, important warnings from the Anomaly Detector surface here —
          the agent itself never pushes.</p>
      </div>
    </Group>
  </SectionGrid>
</Page>

<style>
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
  :global(.spin) {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
