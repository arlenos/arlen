<script lang="ts">
  /// Anomaly notices on the shared ledger row grid, with the honest
  /// placeholder: the agent itself never pushes, only the Anomaly Detector's
  /// rare important warnings surface here.
  import { Bell } from "@lucide/svelte";
  import TimelineRow from "./TimelineRow.svelte";
  import { relativeTime } from "$lib/time";
  import type { Notice } from "$lib/ledger";

  let { notices }: { notices: Notice[] | null } = $props();
</script>

{#if !notices || notices.length === 0}
  <div class="placeholder">
    <Bell size={20} strokeWidth={1.5} />
    <p>
      Rare, important warnings from the Anomaly Detector surface here. The agent itself never
      pushes. Nothing to show right now.
    </p>
  </div>
{:else}
  <ul class="list">
    {#each notices as n (n.tsMicros + n.summary)}
      <TimelineRow
        label={n.critical ? "critical" : "notice"}
        tone={n.critical ? "warn" : "info"}
        subject={n.summary}
        detail={n.body ? [{ text: n.body }] : []}
        time={relativeTime(n.tsMicros)}
      />
    {/each}
  </ul>
{/if}

<style>
  .placeholder {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .placeholder p {
    margin: 0;
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
