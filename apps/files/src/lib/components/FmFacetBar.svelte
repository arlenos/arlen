<script lang="ts">
  /// The faceted KG filter bar, revealed under the headerbar when Filter is on.
  /// A row of facet dropdowns (Project / Type / Time / Touched), each a
  /// multi-select of values with a graph count when one is known, builds a
  /// `facet:` query; the chosen values render as dismissible chips below, and
  /// the listing navigates to the faceted result the moment a value toggles.
  /// "Save" names the combo into the sidebar as a Smart Folder; "Clear" drops
  /// every facet and returns to the folder the filter opened over.
  import { get } from "svelte/store";
  import { ChevronDown, X } from "lucide-svelte";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import Dialog from "@arlen/ui-kit/components/ui/dialog/dialog.svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import {
    FACET_GROUPS,
    GROUP_LABEL,
    TYPE_VALUES,
    TIME_VALUES,
    projectValues,
    touchedValues,
    selectedFacets,
    serializeFacets,
    toggleValue,
    clearGroup,
    clearFacets,
    saveSmartFolder,
    type FacetGroup,
    type FacetValue,
  } from "$lib/stores/facets";
  import { t } from "$lib/i18n/messages";

  let {
    basePath,
    onnavigate,
  }: {
    /// The real folder the filter opened over; an empty selection returns here.
    basePath: string;
    /// Route the active controller to a location (the `facet:` key or the base).
    onnavigate: (location: string) => void;
  } = $props();

  // The value set offered by each group: type and time are static, project and
  // touched come from the graph (empty until loaded, or when there are none).
  const valuesFor = (group: FacetGroup): FacetValue[] => {
    if (group === "type") return TYPE_VALUES;
    if (group === "time") return TIME_VALUES;
    if (group === "project") return $projectValues;
    return $touchedValues;
  };

  // Resolve a selected value key back to its label for the chip text.
  const labelOf = (group: FacetGroup, value: string): string =>
    valuesFor(group).find((v) => v.value === value)?.label ?? value;

  // One chip per group that has a selection: the muted group name plus its
  // chosen value labels, with a single dismiss that drops the whole group.
  const chips = $derived(
    FACET_GROUPS.filter((g) => $selectedFacets[g].size > 0).map((g) => ({
      group: g,
      label: GROUP_LABEL[g],
      values: [...$selectedFacets[g]].map((v) => labelOf(g, v)).join(", "),
    })),
  );

  const anyActive = $derived(chips.length > 0);

  // Navigate to the current facet query, or back to the base when it empties.
  function apply(): void {
    onnavigate(serializeFacets(get(selectedFacets)) || basePath);
  }

  function pick(group: FacetGroup, value: string): void {
    toggleValue(group, value);
    apply();
  }

  function dropGroup(group: FacetGroup): void {
    clearGroup(group);
    apply();
  }

  function clearAll(): void {
    clearFacets();
    apply();
  }

  let saveOpen = $state(false);
  let saveName = $state("");

  function openSave(): void {
    saveName = "";
    saveOpen = true;
  }

  function commitSave(): void {
    saveSmartFolder(saveName);
    saveOpen = false;
  }
</script>

<div class="facet-bar">
  <div class="facet-row">
    {#each FACET_GROUPS as group (group)}
      {@const values = valuesFor(group)}
      {@const count = $selectedFacets[group].size}
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <button
              class="facet-trigger"
              class:on={count > 0}
              aria-label={$t("f.facet.filterByAria", { group: GROUP_LABEL[group].toLowerCase() })}
              {...props}
            >
              <span>{GROUP_LABEL[group]}</span>
              {#if count > 0}<span class="facet-count">{count}</span>{/if}
              <ChevronDown size={12} strokeWidth={2} class="facet-chev" />
            </button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="start" sideOffset={4} class="fm-menu facet-menu">
          {#if values.length === 0}
            <div class="facet-empty">{$t("f.facet.empty")}</div>
          {:else}
            {#each values as v (v.value)}
              <DropdownMenu.CheckboxItem
                checked={$selectedFacets[group].has(v.value)}
                closeOnSelect={false}
                onSelect={() => pick(group, v.value)}
              >
                <span class="facet-opt-label">{v.label}</span>
                {#if v.count !== undefined}<span class="facet-opt-count">{v.count}</span>{/if}
              </DropdownMenu.CheckboxItem>
            {/each}
          {/if}
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    {/each}

    <span class="facet-spacer"></span>

    <Button variant="outline" size="sm" disabled={!anyActive} onclick={openSave}>{$t("f.facet.saveBtn")}</Button>
    <Button variant="ghost" size="sm" disabled={!anyActive} onclick={clearAll}>{$t("f.facet.clearBtn")}</Button>
  </div>

  {#if anyActive}
    <div class="facet-chips">
      {#each chips as chip (chip.group)}
        <span class="facet-chip">
          <span class="facet-chip-group">{chip.label}</span>
          <span class="facet-chip-values">{chip.values}</span>
          <button
            class="facet-chip-x"
            aria-label={$t("f.facet.removeAria", { label: chip.label })}
            onclick={() => dropGroup(chip.group)}
          >
            <X size={11} strokeWidth={2.25} />
          </button>
        </span>
      {/each}
    </div>
  {/if}
</div>

<Dialog open={saveOpen} onClose={() => (saveOpen = false)} size="sm" labelledby="facet-save-title">
  <div class="facet-dialog">
    <h2 id="facet-save-title">{$t("f.facet.saveAsFolder")}</h2>
    <p>{$t("f.facet.saveDesc")}</p>
    <Input
      bind:value={saveName}
      class="h-8 text-sm"
      placeholder={$t("f.facet.folderName")}
      aria-label={$t("f.facet.folderName")}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commitSave();
        }
      }}
    />
    <div class="facet-dialog-actions">
      <Button variant="ghost" onclick={() => (saveOpen = false)}>{$t("f.ops.cancel")}</Button>
      <Button onclick={commitSave}>{$t("f.save")}</Button>
    </div>
  </div>
</Dialog>

<style>
  .facet-bar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .facet-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .facet-spacer {
    flex: 1;
  }

  .facet-trigger {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: var(--height-control, 28px);
    padding: 0 10px;
    border: 1px solid var(--control-border);
    background: var(--control-bg);
    border-radius: var(--radius-input);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }
  .facet-trigger:hover,
  .facet-trigger[aria-expanded="true"] {
    background: var(--control-bg-hover);
  }
  .facet-trigger.on {
    border-color: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .facet-trigger :global(.facet-chev) {
    opacity: 0.55;
  }
  .facet-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    font-size: 0.625rem;
    font-weight: 600;
    line-height: 1;
  }

  /* Save (outline) is the affirmative action; Clear (ghost) is the quiet reset.
     Both are kit Buttons at size sm (28px, flush with the selectors). */

  /* The dropdown value rows carry a trailing count, so a label fills the row
     and the count sits at its end. */
  .facet-opt-label {
    flex: 1;
  }
  .facet-opt-count {
    margin-inline-start: 1rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
  }
  .facet-empty {
    padding: 6px 10px;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-size: 0.75rem;
  }

  /* A hairline parts the selected tags from the selectors above, so a long
     wrapping run of chips stays visually its own band. It bleeds to the bar
     edges (the negative margin cancels the bar padding) so the rule spans the
     full width, while the chips themselves stay aligned with the selectors. */
  .facet-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0 -10px;
    padding: 8px 10px 0;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .facet-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 4px 0 9px;
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    font-size: 0.75rem;
  }
  .facet-chip-group {
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .facet-chip-values {
    color: var(--foreground);
  }
  .facet-chip-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: none;
    border-radius: var(--radius-full);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .facet-chip-x:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
  }

  .facet-dialog {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 20px;
  }
  .facet-dialog h2 {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .facet-dialog p {
    margin-top: -4px;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .facet-dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
