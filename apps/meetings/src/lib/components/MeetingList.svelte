<script lang="ts">
  /// The Meetings home: your recent meetings + the start action. Each row opens its
  /// note (a KG node); Start begins an on-device capture. The whole lifecycle -
  /// home -> capture -> note, home -> open a past meeting - lives off this landing.
  import { meetings, startCapture, openMeeting, fmtDate } from "$lib/stores/meeting";
  import { t, dir } from "$lib/i18n/messages";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Mic } from "lucide-svelte";
</script>

<div class="home" dir={$dir}>
  <header class="home-head">
    <h1 class="home-title">{$t("mt.title")}</h1>
    <Button variant="secondary" size="sm" onclick={() => startCapture()}>
      <Mic size={14} strokeWidth={2} /> {$t("mt.start")}
    </Button>
  </header>

  {#if $meetings.length === 0}
    <p class="empty">{$t("mt.empty")}</p>
  {:else}
    <ul class="list">
      {#each $meetings as m (m.id)}
        <li>
          <button type="button" class="row" onclick={() => openMeeting(m.id)}>
            <span class="row-title">{m.title}</span>
            <span class="row-meta">
              <span class="row-date">{fmtDate(m.date_ms)}</span>
              <span class="row-parts">{m.participants.join(", ")}</span>
            </span>
            <span class="row-preview">{m.preview}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <p class="foot">{$t("mt.foot")}</p>
</div>

<style>
  .home {
    height: 100vh;
    overflow-y: auto;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
    padding: 1.75rem;
  }
  .home-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    max-width: 44rem;
    margin: 0 auto 1.25rem;
  }
  .home-title {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 600;
  }
  .empty {
    max-width: 44rem;
    margin: 2rem auto;
    font-size: 0.875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .list {
    list-style: none;
    margin: 0 auto;
    padding: 0;
    max-width: 44rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    width: 100%;
    padding: 0.75rem 0.85rem;
    border: none;
    border-radius: var(--radius-card, 12px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .row-title {
    font-size: 0.9375rem;
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .row-meta {
    display: flex;
    gap: 1rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .row-date {
    font-variant-numeric: tabular-nums;
  }
  .row-preview {
    font-size: 0.8125rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--color-fg-primary) 62%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .foot {
    max-width: 44rem;
    margin: 1.5rem auto 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
  }
</style>
