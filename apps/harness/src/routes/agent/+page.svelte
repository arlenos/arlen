<script lang="ts">
  /// The Activity feed (harness-redo-plan.md, decided 11 June): the
  /// review-only view of what the AI did for you, newest first, with
  /// per-entry undo. Not a peer mode to Chat; it opens from the sidebar's
  /// quiet Activity entry. Configuration (master switch, posture,
  /// behaviours) lives in Settings, not here. This route owns the reads,
  /// the polling, the filter state, and the undo call; rendering lives in
  /// `$lib/components/agent`.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import ActivityTimeline from "$lib/components/agent/ActivityTimeline.svelte";
  import WarningsPanel from "$lib/components/agent/WarningsPanel.svelte";
  import ExplainPanel from "$lib/components/agent/ExplainPanel.svelte";
  import { readCapability, type Capability } from "$lib/capability";
  import { categorize } from "$lib/display";
  import type { ActivityEntry, ActivityPage, Notice, NoticesResult } from "$lib/ledger";

  let activity = $state<ActivityPage | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let notices = $state<Notice[] | null>(null);
  // An unreadable warning source is shown as exactly that, never as the
  // all-clear (`available: false` from the read).
  let noticesUnreadable = $state(false);
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

  // Feed filters. User-driven `$state` (not an IPC callback), so plain
  // reactivity is reliable here.
  let category = $state("all");
  let outcome = $state("all");
  let timeWindow = $state("all");

  // How much of the record is loaded; "Show older entries" widens the
  // window through the same read command.
  const PAGE = 100;
  let loadLimit = $state(PAGE);
  function loadMore() {
    loadLimit += PAGE;
    load();
  }

  const WINDOW_MS: Record<string, number | null> = {
    all: null,
    "1h": 3_600_000,
    "24h": 86_400_000,
    "7d": 604_800_000,
  };

  // The visible feed after the filters. Time compares against the wall
  // clock; entry timestamps are microseconds, the window is milliseconds.
  const filteredEntries = $derived.by(() => {
    const entries = activity?.entries ?? [];
    const windowMs = WINDOW_MS[timeWindow] ?? null;
    const now = Date.now();
    return entries.filter(
      (e) =>
        (category === "all" || categorize(e.kind).key === category) &&
        (outcome === "all" || e.outcome === outcome) &&
        (windowMs === null || now - e.timestampMicros / 1000 <= windowMs),
    );
  });

  /// Undo one change through the agent's compensation path. Undo targets the
  /// action's correlation id, which the audit carries as the entry's call-chain
  /// id; the registered `undo_action(id)` command forwards it to the agent's
  /// `compensate`. An entry without a call-chain id is not an undoable action.
  async function undoEntry(entry: ActivityEntry): Promise<boolean> {
    if (!entry.callChainId) return false;
    try {
      const status = await invoke<string>("undo_action", { id: entry.callChainId });
      // The compensation lands as a new ledger entry; refresh so it shows.
      refreshLive();
      // Only a real retract (or an already-gone write) counts as undone.
      return status === "retracted" || status === "nothing-to-undo";
    } catch {
      return false;
    }
  }

  // Monotonic token for live refreshes. A manual `load()` and each background
  // poll bump it; a poll applies its result only if still the latest, so a slow
  // poll can never overwrite newer state (a manual reload, or a later poll).
  let refreshSeq = 0;
  // True when the last live poll could not reach a source, so the feed /
  // warnings on screen may be stale. Surfaced in the UI, since silently
  // showing old data on a review surface could hide an outage or a missed
  // critical warning.
  let liveStale = $state(false);

  async function load() {
    loading = true;
    error = null;
    // Invalidate any in-flight background poll so its (older) result cannot land
    // after this authoritative reload.
    refreshSeq++;
    // The capability read only drives the Explain affordance; it loads
    // independently so an outage never blanks the feed.
    capability = await readCapability();
    try {
      const n = await invoke<NoticesResult>("ai_notices");
      notices = n.notices;
      noticesUnreadable = !n.available;
    } catch {
      notices = null;
      noticesUnreadable = true;
    }
    try {
      activity = await invoke<ActivityPage>("ai_activity_recent", { limit: loadLimit });
      liveStale = false;
    } catch (e) {
      error = String(e);
      activity = null;
    } finally {
      loading = false;
    }
  }

  // Silent background refresh of the live elements: the feed (new entries)
  // and the warnings (time-sensitive). No spinner flicker; a transient poll
  // failure keeps the current view and flags staleness instead of blanking
  // it. The manual Refresh reloads everything and reports real errors.
  async function refreshLive() {
    if (loading) return;
    const seq = ++refreshSeq;
    let failed = false;
    let nextActivity: ActivityPage | null = null;
    let nextNotices: NoticesResult | null = null;
    try {
      const a = await invoke<ActivityPage>("ai_activity_recent", { limit: loadLimit });
      // A successful call can still report the record unreadable
      // (available=false). That is a degraded poll, not fresh data: keep the
      // last-known feed and flag staleness rather than blanking it.
      if (a.available) nextActivity = a;
      else failed = true;
    } catch {
      failed = true;
    }
    try {
      const n = await invoke<NoticesResult>("ai_notices");
      if (n.available) nextNotices = n;
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
      notices = nextNotices.notices;
      noticesUnreadable = false;
    }
    // A failed poll means what is on screen may be stale; surface that rather
    // than letting old data look current. A later good poll clears it.
    liveStale = failed;
  }

  const REFRESH_MS = 10_000;

  onMount(() => {
    load();
    // Poll the live elements so new entries and warnings appear without a
    // manual refresh. Paused while the window is hidden; the next visible
    // tick catches up.
    const timer = setInterval(() => {
      if (!document.hidden) refreshLive();
    }, REFRESH_MS);
    return () => clearInterval(timer);
  });
</script>

<Page
  title="Activity"
  description="Everything the assistant did for you on this device, newest first."
>
  <SectionGrid>
    <Group label="Warnings" class="span-full">
      <WarningsPanel {notices} unreadable={noticesUnreadable} />
    </Group>

    <Group class="span-full">
      <ActivityTimeline
        {activity}
        entries={filteredEntries}
        {error}
        {loading}
        {liveStale}
        bind:category
        bind:outcome
        bind:timeWindow
        onrefresh={load}
        onmore={loadMore}
        onundo={undoEntry}
      />
    </Group>

    <Group label="What's happening now" class="span-full">
      <ExplainPanel
        {explanation}
        error={explainError}
        busy={explaining}
        aiOff={capability !== null && !capability.enabled}
        onexplain={runExplain}
      />
    </Group>
  </SectionGrid>
</Page>
