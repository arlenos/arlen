<script lang="ts">
  /// The Projects browser (KA-R3): the kit Miller columns over the
  /// hierarchical projects adapter - project, members, one member's
  /// relationship hops - never a graph view (decision 4). The head carries the
  /// as-of entry (Tim's pick): a quiet "As of…" button that becomes a dated
  /// chip with x while active; the whole surface wears a thin time edge and
  /// the columns answer for that moment.
  import { onMount } from "svelte";
  import { CalendarClock, X } from "lucide-svelte";
  import {
    createBrowserState,
    FileBrowser,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { projectsAdapter, projectsMocked, asOf, asOfCandidates } from "$lib/stores/projects";
  import { dayLabel } from "$lib/stores/timeline";
  import { t, locale } from "$lib/i18n/messages";

  let {
    onselect,
  }: {
    /// Selection change: the selected entry + the column path it lives in.
    onselect: (entry: FileEntry | null, path: string) => void;
  } = $props();

  const ctrl = createBrowserState(projectsAdapter, { initial: "/projects", root: "/projects" });
  ctrl.viewMode.set("miller");
  const path = ctrl.path;

  const now = Date.now();

  function onselection(entries: FileEntry[]): void {
    onselect(entries[0] ?? null, $path);
  }
  function onactivate(entry: FileEntry): void {
    onselect(entry, $path);
  }

  // The as-of picker: a small popover with recent moments (the fixture's
  // horizon; live this becomes the graph's recorded range).
  let pickerOpen = $state(false);

  // Changing the moment returns to the project list: a drilled path may not
  // exist at the new moment, and the Miller ancestor columns only refetch on
  // a path change - jumping to the root keeps every visible column honest.
  async function retime(unix: number | null): Promise<void> {
    asOf.set(unix);
    pickerOpen = false;
    await ctrl.refresh();
    if ($path !== "/projects") await ctrl.navigate("/projects");
    onselect(null, "/projects");
  }
  function pick(unix: number): void {
    void retime(unix);
  }
  function clearAsOf(): void {
    void retime(null);
  }

  function chipLabel(unix: number): string {
    return new Date(unix * 1000).toLocaleDateString($locale, { weekday: "short", day: "numeric", month: "short" });
  }
  // Local midnight for the picker rows, so "Yesterday" matches dayLabel's
  // local-day comparison (a raw unix floor is UTC and can miss by a day).
  function localDay(unix: number): number {
    const d = new Date(unix * 1000);
    d.setHours(0, 0, 0, 0);
    return Math.floor(d.getTime() / 1000);
  }

  onMount(() => () => asOf.set(null));
</script>

<div class="pr" class:timeTravel={$asOf !== null}>
  <div class="pr-head">
    {#if $projectsMocked}
      <span class="pr-sample">{$t("k.sample")}</span>
    {/if}
    <span class="pr-spacer"></span>
    {#if $asOf === null}
      <div class="pr-asof-wrap">
        <button type="button" class="pr-asof" onclick={() => (pickerOpen = !pickerOpen)}>
          <CalendarClock size={13} strokeWidth={2} />
          {$t("k.pr.asof")}
        </button>
        {#if pickerOpen}
          <div class="pr-picker" role="listbox" aria-label={$t("k.pr.asofAria")}>
            {#each asOfCandidates() as cand (cand)}
              <button type="button" class="pr-pick" role="option" aria-selected="false" onclick={() => pick(cand)}>
                {dayLabel(localDay(cand), $locale)}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      <span class="pr-chip">
        <CalendarClock size={12} strokeWidth={2} />
        {$t("k.pr.asofActive", { day: chipLabel($asOf) })}
        <button type="button" class="pr-chip-x" onclick={clearAsOf} aria-label={$t("k.pr.asofClear")}>
          <X size={12} strokeWidth={2} />
        </button>
      </span>
    {/if}
  </div>

  {#if $asOf !== null}
    <div class="pr-edge" aria-hidden="true"></div>
  {/if}

  <div class="pr-columns">
    <FileBrowser controller={ctrl} {onselection} {onactivate} {now} nameLabel={$t("k.place.projects")} emptyLabel={$t("k.empty.projects")} />
  </div>
</div>

<style>
  .pr {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .pr-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 1.1rem 0.45rem;
  }
  .pr-sample {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .pr-spacer {
    flex: 1;
  }

  .pr-asof-wrap {
    position: relative;
  }
  .pr-asof {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.55rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    cursor: pointer;
  }
  .pr-asof:hover {
    color: var(--color-fg-primary);
  }
  .pr-picker {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    min-width: 13rem;
    padding: 0.25rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    border-radius: var(--radius-input);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg);
  }
  .pr-pick {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.375rem 0.5rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font-size: var(--text-xs);
    color: var(--color-fg-primary);
    text-align: start;
    cursor: pointer;
  }
  .pr-pick:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 7%, transparent);
  }

  /* Active time travel: the chip names the moment; the x returns to now. */
  .pr-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.35rem 0.25rem 0.55rem;
    border: 1px solid color-mix(in srgb, var(--color-accent, #6aa9e0) 40%, transparent);
    border-radius: var(--radius-button, 6px);
    background: color-mix(in srgb, var(--color-accent, #6aa9e0) 10%, transparent);
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .pr-chip-x {
    display: inline-flex;
    padding: 0.125rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    cursor: pointer;
  }
  .pr-chip-x:hover {
    color: var(--color-fg-primary);
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
  }

  /* The thin time edge under the head while the surface answers for the past. */
  .pr-edge {
    height: 2px;
    margin: 0 1.1rem;
    border-radius: 1px;
    background: color-mix(in srgb, var(--color-accent, #6aa9e0) 55%, transparent);
  }

  .pr-columns {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
