<script lang="ts">
  /// The AI transparency surface (ai-transparency-surface.md, Gap 3): the
  /// one place that shows, in plain language, what the AI can reach, has
  /// read, is holding, did, costs, and how to turn it off. AI-scoped only
  /// (kg-surface-allocation.md): never the whole machine's authority. The
  /// dashboard archetype, a single-column story from access to off switch.
  /// This route owns the reads; rendering lives in `$lib/components/transparency`.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import AccessSection from "$lib/components/transparency/AccessSection.svelte";
  import ReadsSection from "$lib/components/transparency/ReadsSection.svelte";
  import MemorySection from "$lib/components/transparency/MemorySection.svelte";
  import ActivitySection from "$lib/components/transparency/ActivitySection.svelte";
  import CostSection from "$lib/components/transparency/CostSection.svelte";
  import OffSwitchSection from "$lib/components/transparency/OffSwitchSection.svelte";
  import { readCapability, type Capability } from "$lib/capability";
  import { readGrants, readWorkingSet, type GrantView, type WorkingSet } from "$lib/transparency";
  import type { ActivityEntry, ActivityPage } from "$lib/ledger";

  // Direct-assigned awaited reads (the agent surface's proven pattern):
  // each load settles independently so one outage never blanks the rest.
  let capability = $state<Capability | null>(null);
  let grants = $state<GrantView[] | null>(null);
  let workingSet = $state<WorkingSet | null>(null);
  let activity = $state<ActivityPage | null>(null);
  let reads = $state<ActivityPage | null>(null);

  let capLoaded = $state(false);
  let grantsLoaded = $state(false);
  let memoryLoaded = $state(false);
  let activityLoaded = $state(false);
  let readsLoaded = $state(false);

  // The Reads section shows a recent slice (the anti-Recall payoff is seeing
  // what was read); the section reports how many more the ledger holds.
  const READS_SLICE = 6;
  const readEntries = $derived.by((): ActivityEntry[] =>
    (reads?.entries ?? []).slice(0, READS_SLICE),
  );

  // The Activity section is a compact slice: the most recent things the
  // autonomous background agent actually did, newest first. The full
  // filterable record is the Activity surface this links to.
  const AGENT_SLICE = 4;
  const agentEntries = $derived.by((): ActivityEntry[] =>
    (activity?.entries ?? []).filter((e) => e.actor === "ai-agent").slice(0, AGENT_SLICE),
  );

  /// Undo one change through the agent's compensation path, then reload the
  /// record so the compensation entry shows. Mirrors the Activity surface.
  async function undoEntry(entry: ActivityEntry): Promise<boolean> {
    try {
      await invoke("ai_undo", { entryRef: entry.entryRef });
      activity = await invoke<ActivityPage>("ai_activity_recent", { limit: 100 });
      return true;
    } catch {
      return false;
    }
  }

  onMount(async () => {
    readCapability().then((c) => {
      capability = c;
      capLoaded = true;
    });
    readGrants().then((g) => {
      grants = g;
      grantsLoaded = true;
    });
    readWorkingSet().then((w) => {
      workingSet = w;
      memoryLoaded = true;
    });
    invoke<ActivityPage>("ai_activity_recent", { limit: 100 })
      .then((a) => (activity = a))
      .catch(() => (activity = null))
      .finally(() => (activityLoaded = true));
    invoke<ActivityPage>("ai_reads_recent", { limit: 100 })
      .then((r) => (reads = r))
      .catch(() => (reads = null))
      .finally(() => (readsLoaded = true));
  });
</script>

<Page
  title="Transparency"
  description="What the assistant can reach, has read, is holding, did, what it costs, and how to turn it off."
>
  <SectionGrid>
    <Group label="What it can reach" class="span-full">
      <AccessSection {grants} {capability} loaded={capLoaded && grantsLoaded} />
    </Group>

    <Group label="What it has read" class="span-full">
      <ReadsSection {reads} entries={readEntries} loaded={readsLoaded} {capability} />
    </Group>

    <Group label="What it is holding now" class="span-full">
      <MemorySection {workingSet} {capability} loaded={memoryLoaded} />
    </Group>

    <Group label="What it did" class="span-full">
      <ActivitySection
        {activity}
        entries={agentEntries}
        loaded={activityLoaded}
        onundo={undoEntry}
        onseeall={() => goto("/agent")}
      />
    </Group>

    <Group label="What it costs" class="span-full">
      <CostSection {capability} loaded={capLoaded} />
    </Group>

    <Group label="Turning it off" class="span-full">
      <OffSwitchSection {capability} />
    </Group>
  </SectionGrid>
</Page>
