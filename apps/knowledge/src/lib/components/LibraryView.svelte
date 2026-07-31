<script lang="ts">
  /// The Library (decision 7 / §3b): the bridged knowledge content in one
  /// list, one section per source. The section head carries the class label
  /// and, quietly, the origin tag with its count (comma-separated, never a
  /// middot) - the same origin a per-source revoke would sever. Rows reuse
  /// the search anatomy: emphasized title, quiet sub, time in a FIXED column
  /// so nothing drifts between rows.
  import { onMount } from "svelte";
  import { sources, libraryMocked, loadLibrary, type LibraryEntry } from "$lib/stores/library";
  import { t, locale } from "$lib/i18n/messages";

  let { onselect }: { onselect: (e: LibraryEntry) => void } = $props();

  onMount(loadLibrary);

  function dayName(at: number): string {
    return new Date(at * 1000).toLocaleDateString($locale, { day: "numeric", month: "short" });
  }
</script>

<div class="li">
  <div class="li-head">
    {#if $libraryMocked}
      <span class="li-sample">{$t("k.sample")}</span>
    {/if}
  </div>

  <div class="li-scroll">
    {#if $sources && $sources.length === 0}
      <p class="li-empty">{$t("k.empty.library")}</p>
    {:else if $sources}
      {#each $sources as src (src.key)}
        <section class="li-source">
          <h2 class="li-source-head">
            <span class="li-source-label">{$t(`k.li.${src.key}`)}</span>
            <span class="li-source-origin">{$t("k.li.origin", { bridge: src.bridge, n: src.entries.length })}</span>
          </h2>
          {#each src.entries as e (e.id)}
            <button type="button" class="li-row" onclick={() => onselect(e)}>
              <span class="li-title">{e.title}</span>
              <span class="li-sub">{e.sub}</span>
              <span class="li-time">{dayName(e.at)}</span>
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
  .li-sample {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
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
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .li-source-origin {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
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
    cursor: pointer;
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
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .li-time {
    justify-self: end;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    white-space: nowrap;
  }
</style>
