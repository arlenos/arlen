<script lang="ts">
  /// Warnings from the anomaly watcher, on the shared ledger row grid. The
  /// AI itself never pushes; only these rare, important warnings surface.
  /// An unreadable warning source is shown as exactly that, never as the
  /// all-clear.
  import { t } from "$lib/i18n/messages";
  import TimelineRow from "./TimelineRow.svelte";
  import { relativeTime } from "$lib/time";
  import type { Notice } from "$lib/ledger";

  let {
    notices,
    unreadable,
  }: {
    /// The loaded warnings; `null` before the first read settles.
    notices: Notice[] | null;
    /// True when the warning source could not be read.
    unreadable: boolean;
  } = $props();
</script>

{#if unreadable}
  <p class="empty">{$t("h.warnings.cantRead")}</p>
{:else if !notices || notices.length === 0}
  <p class="empty">{$t("h.warnings.none")}</p>
{:else}
  <ul class="list">
    {#each notices as n (n.tsMicros + n.summary)}
      <TimelineRow
        label={n.critical ? $t("h.warnings.warning") : $t("h.warnings.notice")}
        tone={n.critical ? "warn" : "info"}
        subject={n.summary}
        detail={n.body ? [{ text: n.body }] : []}
        time={relativeTime(n.tsMicros)}
      />
    {/each}
  </ul>
{/if}

<style>
  .empty {
    margin: 0;
    padding: 0.75rem var(--space-row, 0.75rem);
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  /* Row dividers live on the list, since sibling rows are separate component
     instances the row's own scoped CSS cannot pair. */
  .list :global(li + li) {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
</style>
