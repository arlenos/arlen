<script lang="ts">
  /// The transparency drawer: the ONE accountability surface (it replaced the
  /// standalone /transparency page, so there is a single implementation). It
  /// slides over the conversation, summoned from the composer foot - the
  /// "glance-away, not away-from" answer to Gap 3. It composes the same real,
  /// wired section components the page used (Access/Grants, Reads, Memory,
  /// Activity, Cost, the off-switch), in a tight drawer frame. Activity is a
  /// compact slice here with "see everything" linking to the full /agent list.
  /// Loads its feeds when it opens; each read settles independently so one
  /// outage never blanks the rest.
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { X } from "@lucide/svelte";
  import AccessSection from "$lib/components/transparency/AccessSection.svelte";
  import ReadsSection from "$lib/components/transparency/ReadsSection.svelte";
  import MemorySection from "$lib/components/transparency/MemorySection.svelte";
  import ActivitySection from "$lib/components/transparency/ActivitySection.svelte";
  import CostSection, { type Usage } from "$lib/components/transparency/CostSection.svelte";
  import OffSwitchSection from "$lib/components/transparency/OffSwitchSection.svelte";
  import { readCapability, type Capability } from "$lib/capability";
  import { readGrants, readWorkingSet, type GrantView, type WorkingSet } from "$lib/transparency";
  import type { ActivityEntry, ActivityPage } from "$lib/ledger";
  import { transparencyOpen } from "$lib/stores/transparency";

  let capability = $state<Capability | null>(null);
  let grants = $state<GrantView[] | null>(null);
  let workingSet = $state<WorkingSet | null>(null);
  let activity = $state<ActivityPage | null>(null);
  let reads = $state<ActivityPage | null>(null);
  let usage = $state<Usage | null>(null);

  let capLoaded = $state(false);
  let grantsLoaded = $state(false);
  let memoryLoaded = $state(false);
  let activityLoaded = $state(false);
  let readsLoaded = $state(false);
  let usageLoaded = $state(false);

  // Compact slices: the drawer shows the most recent few; the full filterable
  // record is the /agent surface the Activity section links to.
  const AGENT_SLICE = 4;
  const READS_SLICE = 5;
  const agentEntries = $derived.by((): ActivityEntry[] =>
    (activity?.entries ?? []).filter((e) => e.actor === "ai-agent").slice(0, AGENT_SLICE),
  );
  const readEntries = $derived.by((): ActivityEntry[] =>
    (reads?.entries ?? []).slice(0, READS_SLICE),
  );

  let loadedOnce = false;
  function load() {
    readCapability().then((c) => ((capability = c), (capLoaded = true)));
    readGrants().then((g) => ((grants = g), (grantsLoaded = true)));
    readWorkingSet().then((w) => ((workingSet = w), (memoryLoaded = true)));
    invoke<ActivityPage>("ai_activity_recent", { limit: 100 })
      .then((a) => (activity = a))
      .catch(() => (activity = null))
      .finally(() => (activityLoaded = true));
    invoke<ActivityPage>("ai_reads_recent", { limit: 100 })
      .then((r) => (reads = r))
      .catch(() => (reads = null))
      .finally(() => (readsLoaded = true));
    invoke<string>("ai_usage")
      .then((json) => {
        try {
          const u = JSON.parse(json) as { totalTokens?: number };
          usage = { totalTokens: u.totalTokens ?? 0 };
        } catch {
          usage = null;
        }
      })
      .catch(() => (usage = null))
      .finally(() => (usageLoaded = true));
  }

  // Load on first open and refresh on every subsequent open, so a returning
  // user sees current reads/activity without a manual refresh.
  $effect(() => {
    if ($transparencyOpen) {
      load();
      loadedOnce = true;
    }
  });

  async function undoEntry(entry: ActivityEntry): Promise<boolean> {
    // Undo targets the action's correlation id, which the agent's audit carries
    // as the entry's call-chain id (`behaviour_action_event`). The registered
    // command is `undo_action(id)`, which forwards it to the agent's
    // `compensate`; an entry without a call-chain id is not an undoable action.
    if (!entry.callChainId) return false;
    try {
      const status = await invoke<string>("undo_action", { id: entry.callChainId });
      activity = await invoke<ActivityPage>("ai_activity_recent", { limit: 100 });
      // The agent answers with a status; only a real retract (or an already-gone
      // write) counts as undone. `not-enabled` / `no-such-receipt` / `error:` do not.
      return status === "retracted" || status === "nothing-to-undo";
    } catch {
      return false;
    }
  }

  function close() {
    transparencyOpen.set(false);
  }
  function seeAllActivity() {
    close();
    goto("/agent");
  }

  // Escape closes the drawer, like any overlay.
  onMount(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && $transparencyOpen) {
        e.preventDefault();
        close();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });
</script>

{#if $transparencyOpen}
  <div class="scrim" onclick={close} role="presentation"></div>
  <aside class="drawer" aria-label={$t("h.transparency.aria")}>
    <header class="head">
      <span class="head-title">{$t("h.transparency.title")}</span>
      <button class="x" aria-label={$t("h.transparency.close")} onclick={close}>
        <X size={15} strokeWidth={2} />
      </button>
    </header>

    <div class="body">
      <section class="sec">
        <div class="sec-head">{$t("h.transparency.reach")}</div>
        <AccessSection {grants} {capability} loaded={capLoaded && grantsLoaded} />
      </section>

      <section class="sec">
        <div class="sec-head">{$t("h.transparency.read")}</div>
        <ReadsSection {reads} entries={readEntries} loaded={readsLoaded} {capability} />
      </section>

      <section class="sec">
        <div class="sec-head">{$t("h.transparency.holding")}</div>
        <MemorySection {workingSet} {capability} loaded={memoryLoaded} />
      </section>

      <section class="sec">
        <div class="sec-head">{$t("h.transparency.did")}</div>
        <ActivitySection
          {activity}
          entries={agentEntries}
          loaded={activityLoaded}
          onundo={undoEntry}
          onseeall={seeAllActivity}
        />
      </section>

      <section class="sec">
        <div class="sec-head">{$t("h.transparency.costs")}</div>
        <CostSection {capability} {usage} loaded={capLoaded && usageLoaded} />
      </section>
    </div>

    <div class="off">
      <OffSwitchSection {capability} />
    </div>
  </aside>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: color-mix(in srgb, #000 45%, transparent);
  }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 21;
    width: 24rem;
    max-width: 92vw;
    display: flex;
    flex-direction: column;
    background: var(--color-bg-app, #0f0f0f);
    border-left: 1px solid var(--color-border);
    box-shadow: -12px 0 40px rgba(0, 0, 0, 0.4);
    font-size: var(--text-base);
    color: var(--foreground, #fafafa);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 44px;
    padding: 0 0.5rem 0 1rem;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .head-title {
    font-size: var(--text-sm);
    font-weight: 500;
  }
  .x {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    border-radius: var(--radius-button, 6px);
    cursor: pointer;
  }
  .x:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  /* The sections scroll; the off-switch stays pinned at the foot. */
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .sec {
    padding: 0.625rem 1rem 0.75rem;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  /* The tight uppercase section header (the dense drawer frame); the section
     component renders its real content below it. */
  .sec-head {
    margin-bottom: 0.375rem;
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .off {
    flex-shrink: 0;
    padding: 0.5rem;
    border-top: 1px solid var(--color-border);
  }
</style>
