<script lang="ts">
  /// Agent dashboard (ai-app.md §2.2) — the pull / observability surface:
  /// the read-only activity timeline from the tamper-evident audit ledger,
  /// behaviour status, anomaly notices, and System Explanation Mode. This
  /// route owns the reads, the polling, and the filter state; rendering
  /// lives in the `$lib/components/agent` family.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import PostureBanner from "$lib/components/agent/PostureBanner.svelte";
  import ActivityTimeline from "$lib/components/agent/ActivityTimeline.svelte";
  import BehaviourPanel from "$lib/components/agent/BehaviourPanel.svelte";
  import NoticePanel from "$lib/components/agent/NoticePanel.svelte";
  import ExplainPanel from "$lib/components/agent/ExplainPanel.svelte";
  import AgentFilters from "$lib/components/agent/AgentFilters.svelte";
  import { readCapability, type Capability } from "$lib/capability";
  import {
    KIND_META,
    type ActivityPage,
    type BehaviourReport,
    type Notice,
    type NoticesResult,
  } from "$lib/ledger";

  let activity = $state<ActivityPage | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let behaviours = $state<BehaviourReport | null>(null);
  let notices = $state<Notice[] | null>(null);
  let capability = $state<Capability | null>(null);

  // System Explanation Mode (Foundation §5.8), generated on demand.
  let explanation = $state<string | null>(null);
  let explainError = $state<string | null>(null);
  let explaining = $state(false);

  async function runExplain() {
    explaining = true;
    explainError = null;
    try {
      explanation = await invoke<string>("ai_explain");
    } catch (e) {
      explainError = String(e);
      explanation = null;
    } finally {
      explaining = false;
    }
  }

  // Activity-timeline filters. User-driven `$state` (not an IPC callback),
  // so plain reactivity is reliable here.
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

  // Monotonic token for live refreshes. A manual `load()` and each background
  // poll bump it; a poll applies its result only if still the latest, so a slow
  // poll can never overwrite newer state (a manual reload, or a later poll).
  let refreshSeq = 0;
  // True when the last live poll could not reach a source, so the activity /
  // notices on screen may be stale. Surfaced in the UI, since silently showing
  // old data on an observability surface could hide a daemon outage or a missed
  // critical notice.
  let liveStale = $state(false);

  async function load() {
    loading = true;
    error = null;
    // Invalidate any in-flight background poll so its (older) result cannot land
    // after this authoritative reload.
    refreshSeq++;
    // Behaviour status is independent of the audit ledger: load it
    // best-effort so an audit-daemon outage does not blank the behaviour
    // list, and vice versa.
    try {
      behaviours = await invoke<BehaviourReport>("ai_behaviours");
    } catch {
      behaviours = null;
    }
    try {
      notices = (await invoke<NoticesResult>("ai_notices")).notices;
    } catch {
      notices = null;
    }
    // The acting posture (executor_live) is config, not ledger state, so
    // it loads independently and a daemon outage never blanks it.
    capability = await readCapability();
    try {
      activity = await invoke<ActivityPage>("ai_activity_recent", { limit: 100 });
      liveStale = false;
    } catch (e) {
      error = String(e);
      activity = null;
    } finally {
      loading = false;
    }
  }

  // Silent background refresh of the live elements: the activity timeline (new
  // audit entries) and the notices (anomaly warnings, which are time-sensitive
  // and should not wait for a manual refresh). No spinner flicker, and a
  // transient poll failure keeps the current view rather than blanking it or
  // surfacing a blip; the manual Refresh button reports real errors and reloads
  // everything (including the rarely-changing behaviour and capability panels).
  async function refreshLive() {
    if (loading) return;
    const seq = ++refreshSeq;
    let failed = false;
    let nextActivity: ActivityPage | null = null;
    let nextNotices: Notice[] | null = null;
    try {
      const a = await invoke<ActivityPage>("ai_activity_recent", { limit: 100 });
      // A successful call can still report the audit daemon unreachable
      // (available=false). That is a degraded poll, not fresh data: keep the
      // last-known timeline and flag staleness rather than blanking it.
      if (a.available) nextActivity = a;
      else failed = true;
    } catch {
      failed = true;
    }
    try {
      const n = await invoke<NoticesResult>("ai_notices");
      // Likewise, an unreadable/malformed alert log returns available=false;
      // do not clear a shown notice and present a degraded source as all-clear.
      if (n.available) nextNotices = n.notices;
      else failed = true;
    } catch {
      failed = true;
    }
    // Latest-wins: drop this result if a newer poll or a manual reload started
    // while it was in flight, so an older response cannot clobber newer state.
    if (seq !== refreshSeq || loading) return;
    if (nextActivity !== null) {
      activity = nextActivity;
      error = null;
    }
    if (nextNotices !== null) {
      notices = nextNotices;
    }
    // A failed poll means what is on screen may be stale; surface that rather
    // than letting old data look current. A later good poll clears it.
    liveStale = failed;
  }

  const REFRESH_MS = 10_000;

  onMount(() => {
    load();
    // Poll the live elements so a running system's new audit entries and
    // anomaly notices appear without a manual refresh. Paused while the window
    // is hidden, so an unseen tab does not poll; the next visible tick catches up.
    const timer = setInterval(() => {
      if (!document.hidden) refreshLive();
    }, REFRESH_MS);
    return () => clearInterval(timer);
  });
</script>

<div class="agent-shell">
  <div class="agent-main">
    <Page
      title="Agent"
      description="What the assistant has done on your behalf. Read-only, from the tamper-evident audit ledger — review each curated action and undo it if you want."
    >
      <SectionGrid>
        {#if capability}
          <PostureBanner {capability} />
        {/if}

        <Group label="Activity" class="span-full">
          <ActivityTimeline
            {activity}
            entries={filteredEntries}
            {error}
            {loading}
            {liveStale}
            onrefresh={load}
          />
        </Group>

        <Group label="Behaviours">
          <BehaviourPanel report={behaviours} />
        </Group>

        <Group label="Notices">
          <NoticePanel {notices} />
        </Group>

        <Group label="What's happening now">
          <ExplainPanel
            {explanation}
            error={explainError}
            busy={explaining}
            onexplain={runExplain}
          />
        </Group>
      </SectionGrid>
    </Page>
  </div>
  <AgentFilters
    kinds={kindOptions}
    outcomes={outcomeOptions}
    bind:selectedKind
    bind:selectedOutcome
    bind:timeWindow
  />
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
</style>
