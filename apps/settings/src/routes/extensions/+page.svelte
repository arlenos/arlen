<script lang="ts">
  /// Everything that extends this system, on one surface
  /// (shell-extension-model.md: apps, shell modules and bridges as facets of one
  /// list, never three UIs). Each row: what it is, where it came from, whether
  /// it is running, what it may reach; the row leads to the thing's own page,
  /// where the reach is read against what the machine saw and can be taken back.
  ///
  /// The kind is a facet, not a lane: the three share exactly the questions this
  /// page answers, and what they do not share belongs on the detail page. The
  /// reach filter is exhaustive because every source emits the one vocabulary
  /// (`contracts/extensions`), which is the whole reason this can be one list.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { AppWindow, Puzzle, Cable, ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { t } from "$lib/i18n/messages";
  import {
    extensions,
    inventoryState,
    inventoryMocked,
    loadExtensions,
    reaches,
    type Extension,
    type ExtensionKind,
    type Reach,
  } from "$lib/stores/extensions";
  import { capChip, healthReadout, originLine } from "$lib/extensionWords";

  onMount(() => {
    void loadExtensions();
  });

  let pivot = $state<"all" | ExtensionKind>("all");
  let query = $state("");
  let reach = $state<Reach>("any");
  /// Pivot, search and filter survive a step back from a detail page
  /// (design-system.md §6.9).
  export const snapshot = {
    capture: () => ({ pivot, query, reach }),
    restore: (v: { pivot: "all" | ExtensionKind; query: string; reach: Reach }) => {
      pivot = v.pivot;
      query = v.query;
      reach = v.reach;
    },
  };

  const KINDS: ExtensionKind[] = ["app", "module", "bridge"];
  const ICONS = { app: AppWindow, module: Puzzle, bridge: Cable } as const;
  const SECTION_KEYS = { app: "s.ext.kind.apps", module: "s.ext.kind.modules", bridge: "s.ext.kind.bridges" } as const;
  const EMPTY_KEYS = { app: "s.ext.none.apps", module: "s.ext.none.modules", bridge: "s.ext.none.bridges" } as const;

  const pivotOptions = $derived([
    { value: "all", label: $t("s.ext.pivot.all") },
    { value: "app", label: $t("s.ext.kind.apps") },
    { value: "module", label: $t("s.ext.kind.modules") },
    { value: "bridge", label: $t("s.ext.kind.bridges") },
  ]);
  const reachOptions = $derived([
    { value: "any", label: $t("s.ext.reach.any") },
    { value: "network", label: $t("s.ext.reach.network") },
    { value: "filesystem", label: $t("s.ext.reach.filesystem") },
    { value: "graph", label: $t("s.ext.reach.graph") },
    { value: "clipboard", label: $t("s.ext.reach.clipboard") },
    { value: "notifications", label: $t("s.ext.reach.notifications") },
    { value: "system", label: $t("s.ext.reach.system") },
  ]);

  const visible = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return $extensions.filter(
      (e) =>
        (pivot === "all" || e.kind === pivot) &&
        (reach === "any" || e.capabilities.some((c) => reaches(c, reach))) &&
        (!q || e.name.toLowerCase().includes(q) || e.id.toLowerCase().includes(q)),
    );
  });
  const shownKinds = $derived(pivot === "all" ? KINDS : [pivot]);
  const filtering = $derived(query.trim() !== "" || reach !== "any");

  function open(e: Extension) {
    void goto(`/extensions/${e.kind}/${encodeURIComponent(e.id)}`);
  }
</script>

<Page title={$t("s.ext.title")} description={$t("s.ext.desc")}>
  <SectionGrid>
    {#if $inventoryMocked}
      <Notice tone="neutral" class="span-full" text={$t("s.ext.sample")} />
    {/if}
    {#if $inventoryState === "unreadable"}
      <Notice tone="error" class="span-full" text={$t("s.ext.unreadable")} />
    {/if}

    <div class="tools span-full">
      <SegmentedControl
        options={pivotOptions}
        value={pivot}
        ariaLabel={$t("s.ext.pivotAria")}
        onchange={(v) => (pivot = v as "all" | ExtensionKind)}
      />
      <div class="tools-right">
        <SearchField id="ext-search" bind:value={query} placeholder={$t("s.ext.search")} aria-label={$t("s.ext.search")} />
        <PopoverSelect
          value={reach}
          options={reachOptions}
          ariaLabel={$t("s.ext.reachAria")}
          onchange={(v) => (reach = v as Reach)}
        />
      </div>
    </div>

    {#if $inventoryState === "loading"}
      <p class="quiet span-full">{$t("s.ext.loading")}</p>
    {:else}
      {#each shownKinds as kind (kind)}
        {@const rows = visible.filter((e) => e.kind === kind)}
        {@const Icon = ICONS[kind]}
        <!-- The kind heading only when kinds sit side by side; under a pivot the
             pivot already says it. -->
        <Section label={pivot === "all" ? $t(SECTION_KEYS[kind]) : undefined} class="span-full">
          {#if rows.length === 0}
            <p class="quiet in-card">{filtering ? $t("s.ext.noMatch") : $t(EMPTY_KEYS[kind])}</p>
          {:else}
            {#each rows as e (e.kind + ":" + e.id)}
              {@const health = healthReadout(e.health, $t)}
              <!-- The whole row leads to the extension's page (the /apps list
                   pattern): a stretched button underneath carries the click. -->
              <div class="ext-row">
                <button type="button" class="ext-go" aria-label={e.name} onclick={() => open(e)}></button>
                <span class="ext-icon"><Icon size={16} strokeWidth={1.75} /></span>
                <span class="ext-text">
                  <span class="ext-name">{e.name}</span>
                  <span class="ext-origin">{originLine(e, $t)}</span>
                  {#if e.capabilities.length > 0}
                    <span class="ext-caps">
                      {#each e.capabilities as cap (cap)}
                        <Badge variant="outline">{capChip(cap, $t)}</Badge>
                      {/each}
                    </span>
                  {:else}
                    <span class="ext-caps ext-nothing">{$t("s.ext.cap.none")}</span>
                  {/if}
                </span>
                <span class="ext-health" class:away={health.posture === "away"}>
                  <span class="found" data-posture={health.posture} aria-hidden="true"></span>
                  {health.text}
                </span>
                <span class="ext-chev"><ChevronRight size={14} strokeWidth={2} /></span>
              </div>
            {/each}
          {/if}
        </Section>
      {/each}
    {/if}
  </SectionGrid>
</Page>

<style>
  .tools {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  /* The pivot keeps its width; the search and the filter take the rest and
     drop under it when the rest is under 18rem, so at 720 nothing overlaps. */
  .tools > :global(:first-child) {
    flex-shrink: 0;
  }
  .tools-right {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1 1 18rem;
    justify-content: flex-end;
    min-width: 0;
  }
  .tools-right :global(.sf) {
    flex: 1 1 auto;
    max-width: 16rem;
  }
  .quiet {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .quiet.in-card {
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .ext-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .ext-row + .ext-row {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .ext-row:hover {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .ext-go {
    position: absolute;
    inset: 0;
    border: none;
    background: transparent;
    border-radius: inherit;
  }
  .ext-go:focus-visible {
    outline: 2px solid var(--color-accent, var(--foreground));
    outline-offset: -2px;
  }
  .ext-icon {
    flex-shrink: 0;
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ext-text {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    flex: 1;
    min-width: 0;
  }
  .ext-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .ext-origin {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .ext-caps {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin-top: 0.25rem;
  }
  .ext-nothing {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  /* The health as a value in the house dot family; the row's own controls
     (nothing yet) would sit above the stretched button, this readout is inert. */
  .ext-health {
    position: relative;
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .ext-health.away {
    color: var(--color-warning);
  }
  .found {
    display: inline-block;
    vertical-align: middle;
    margin-inline-end: 0.45rem;
    width: 6px;
    height: 6px;
    border-radius: var(--radius-chip, 4px);
  }
  .found[data-posture="ours"] {
    background: var(--color-success);
  }
  .found[data-posture="off"] {
    background: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .found[data-posture="away"] {
    background: var(--color-warning);
  }
  .found[data-posture="unknown"] {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .ext-chev {
    flex-shrink: 0;
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
</style>
