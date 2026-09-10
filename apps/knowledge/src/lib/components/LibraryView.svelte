<script lang="ts">
  /// The Library (decision 7 / §3b): the bridged knowledge content in one
  /// list, one section per declared type. The head carries the name the TYPE
  /// declared and, quietly, the source with its count (comma-separated, never a
  /// middot) - the same origin a per-source revoke would sever. Rows reuse the
  /// search anatomy: emphasized title, quiet sub, time in a FIXED column so
  /// nothing drifts between rows.
  ///
  /// Every display class renders as this same list today. The set is closed so
  /// that laying media out differently later is a change here rather than a
  /// guess about a type nobody has seen; until that exists, one layout for all
  /// five is the honest state and the class only fixes the section order.
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { onMount } from "svelte";
  import {
    sources,
    libraryMocked,
    libraryUnavailable,
    libraryNoService,
    loadLibrary,
    type LibraryEntry,
  } from "$lib/stores/library";
  import { t, locale } from "$lib/i18n/messages";

  let { onselect }: { onselect: (e: LibraryEntry) => void } = $props();

  // Load on mount, and again whenever this window becomes visible.
  //
  // Same one-shot read the timeline had: the app starts with the session and asks
  // about seven seconds into the boot, so whoever opens the Library later is looking
  // at what the graph held before it held anything.
  //
  // No interval here, deliberately, and the difference from the timeline is the
  // reasoning rather than the shape. The timeline's 30 second tick is earned by the
  // promotion pass, which changes its data on exactly that cadence. Library sources
  // change when something is imported or a bridge is added - events, not a clock -
  // so a tick would re-read identical rows all day to catch something that announces
  // itself by the window being looked at again anyway.
  onMount(() => {
    const refresh = () => {
      if (!document.hidden) void loadLibrary();
    };
    void loadLibrary();
    document.addEventListener("visibilitychange", refresh);
    return () => document.removeEventListener("visibilitychange", refresh);
  });

  function dayName(at: number): string {
    return new Date(at * 1000).toLocaleDateString($locale, { day: "numeric", month: "short" });
  }
</script>

<div class="li">
  <div class="li-head">
    <!-- Only the sample badge here. The unavailable case used to print the same
         sentence in this line AND as the empty-list text below it, and the store
         makes them inseparable: the catch sets `sources` to `[]` and
         `libraryUnavailable` together, so both always rendered. The one below
         stays, because it sits where the missing content would be; a reader
         looking at an empty pane looks at the pane. Seen for the first time on
         16 August - this view lives behind a sidebar click, and the no-backend
         sweep could not click until then. -->
    {#if $libraryMocked}
      <div class="note"><Notice tone="neutral" text={$t("k.sample")} /></div>
    {/if}
  </div>

  <div class="li-scroll">
    {#if $sources && $sources.length === 0}
      <!-- The empty line names where a source comes from, not only that there
           are none. "No sources bridged in yet" on its own is a dead end with a
           "yet" in it: it implies the reader forgot a step and says nothing
           about which. A bridge is an installed extension
           (`ExtensionKind::Bridge`) and the Store is where one is installed
           from, so that is the honest second half. It names the place rather
           than promising a button this page does not have. -->
      <p class="li-empty">{$libraryNoService
          ? $t("k.li.noService")
          : $libraryUnavailable
            ? $t("k.library.unavailable")
            : $t("k.empty.library")}</p>
    {:else if $sources}
      {#each $sources as src (src.type)}
        <section class="li-source">
          <h2 class="li-source-head">
            <!-- The type's own declared name, not a translated one: it is
                 declared by whoever defined the type, which for a bridge is
                 another repository. A type that declared none reads as its
                 qualified identifier, which is plain and true.
                 THE IDENTIFIER IS NOT UPPERCASED. The head's letter-spaced caps
                 suit a category word; an identifier is a string somebody may
                 need to type back, and `md.obsidian.Note` shown as
                 `MD.OBSIDIAN.NOTE` is no longer the thing it names. The label
                 equalling the type IS the daemon's fallback, read back rather
                 than guessed at. -->
            <span class="li-source-label" class:li-verbatim={src.label === src.type}
              >{src.label}</span
            >
            <span class="li-source-origin">{$t("k.li.origin", { source: src.source, n: src.entries.length })}</span>
          </h2>
          {#each src.entries as e (e.id)}
            <!-- WHEN THERE IS NO SUB-LINE THE TITLE TAKES ITS COLUMN. The grid
                 reserved a third of the row for a second line whether or not the
                 type declared one, so an entry with no sub had its title cut
                 while empty space sat beside it - and the entries with no sub are
                 exactly the ones shown by their IDENTIFIER, which is the string a
                 person may need to read back. Found by the render sweep at 720px:
                 `scans/2026-08-14-001.tiff` ellipsed at 171 of 179 pixels, eight
                 short, next to a third of the row holding nothing. -->
            <button type="button" class="li-row" class:li-wide={!e.sub} onclick={() => onselect(e)}>
              <span class="li-title">{e.title}</span>
              {#if e.sub}<span class="li-sub">{e.sub}</span>{/if}
              <span class="li-time">{e.added === null ? "" : dayName(e.added)}</span>
            </button>
          {/each}
        </section>
      {/each}
    {/if}
  </div>
</div>

<style>
  .li {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .li-head {
    display: flex;
    align-items: center;
    padding: 0.6rem 1.1rem 0;
  }
  .note {
    flex: 1 1 100%;
    margin: 0 0 0.6rem;
  }
  .li-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.25rem 1.1rem 1.25rem;
  }
  .li-empty {
    margin: 0.75rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .li-source {
    display: flex;
    flex-direction: column;
  }
  /* The section head: the content class, then the origin tag with its count,
     quietly - the by-source browse IS the origin story. */
  .li-source-head {
    display: flex;
    align-items: baseline;
    gap: 0.625rem;
    margin: 0.9rem 0 0.25rem;
  }
  .li-source-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  /* An identifier standing in for a name that was never declared. */
  .li-source-label.li-verbatim {
    letter-spacing: 0;
    text-transform: none;
  }
  .li-source-origin {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }

  /* Row anatomy shared with search; the time column is fixed so the flexible
     columns resolve identically on every row. */
  .li-row {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) 5rem;
    align-items: baseline;
    column-gap: 0.75rem;
    width: 100%;
    padding: 0.35rem 0.375rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    text-align: start;
  }
  /* No sub-line: the title spans both flexible columns, the time column stays
     where it is so rows still line up. */
  .li-row.li-wide {
    grid-template-columns: minmax(0, 1fr) 5rem;
  }
  .li-row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .li-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .li-sub {
    min-width: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .li-time {
    justify-self: end;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    white-space: nowrap;
  }
</style>
