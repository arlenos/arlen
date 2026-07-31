<script lang="ts">
  /// Home: your recent meetings and the one action that matters. Rows navigate
  /// to the note route; Start opens the capture route - real navigation, so the
  /// list is always one Back away.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t, dir } from "$lib/i18n/messages";
  import { meetings, meetingsMocked, loadMeetings, fmtDate } from "$lib/stores/meeting";

  onMount(loadMeetings);
</script>

<div class="home" dir={$dir}>
  <div class="home-column">
    <header class="home-head">
      <h1 class="home-title">{$t("mt.title")}</h1>
      <Button id="start-meeting" onclick={() => goto("/capture")}>{$t("mt.start")}</Button>
    </header>

    {#if $meetingsMocked}
      <p class="sample">{$t("mt.sample.list")}</p>
    {/if}

    {#if $meetings.length === 0}
      <p class="empty">{$t("mt.empty")}</p>
    {:else}
      <div class="rows">
        {#each $meetings as m (m.id)}
          <button type="button" class="row" id={`meeting-${m.id}`} onclick={() => goto(`/meeting/${m.id}`)}>
            <span class="row-title">{m.title}</span>
            <span class="row-meta">{fmtDate(m.date_ms)}, {m.participants.join(", ")}</span>
            <span class="row-preview">{m.preview}</span>
          </button>
        {/each}
      </div>
    {/if}

  </div>
</div>

<style>
  .home {
    height: 100%;
    overflow-y: auto;
  }
  .home-column {
    width: 100%;
    max-width: 44rem;
    margin: 0 auto;
    padding: 1.75rem 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .home-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }
  .home-title {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .sample {
    margin: 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.8rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
    text-align: start;
    cursor: pointer;
    transition: background var(--duration-fast, 150ms) ease;
  }
  .row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .row-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .row-meta {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .row-preview {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
