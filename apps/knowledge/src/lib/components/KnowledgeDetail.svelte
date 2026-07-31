<script lang="ts">
  /// The on-demand node-detail panel (knowledge-app.md §3.6, scaffold). KA-R1 shows
  /// the selected node's basics; the bounded-neighbourhood node view + deep
  /// provenance lineage are KA-R7.
  import { X } from "lucide-svelte";
  import type { FileEntry } from "@arlen/ui-kit/components/browser";
  import { formatModified } from "@arlen/ui-kit/components/browser";
  import { clock, type TimelineEvent } from "$lib/stores/timeline";
  import type { ProjectInfo } from "$lib/stores/projects";
  import { t, locale } from "$lib/i18n/messages";

  let {
    entry = null,
    event = null,
    project = null,
    onclose,
  }: {
    /// A browsed graph node (the generic places).
    entry?: FileEntry | null;
    /// A timeline event; when set it wins over `entry`.
    event?: TimelineEvent | null;
    /// A selected project; wins over both (the projects browser).
    project?: ProjectInfo | null;
    onclose: () => void;
  } = $props();
</script>

<aside class="kn-detail" aria-label={$t("k.detail.title")}>
  <header class="kn-detail-head">
    <span class="kn-detail-title">{$t("k.detail.title")}</span>
    <button type="button" class="kn-detail-close" onclick={onclose} aria-label={$t("k.close")}>
      <X size={15} strokeWidth={2} />
    </button>
  </header>

  {#if project}
    <div class="kn-detail-name">{project.name}</div>
    <div class="kn-kv">
      <span class="kn-k">{$t("k.detail.members")}</span>
      <span class="kn-v">{$t("k.detail.membersVal", { n: project.memberCount })}</span>
    </div>
    <div class="kn-kv">
      <span class="kn-k">{$t("k.detail.provenance")}</span>
      <span class="kn-v">{$t("k.detail.detected", { when: formatModified(project.detected) })}</span>
    </div>
    {#if project.events.length > 0}
      <div class="kn-kv">
        <span class="kn-k">{$t("k.detail.recent")}</span>
        <div class="kn-recent">
          {#each project.events as e (e.id)}
            <div class="kn-recent-row">
              <span class="kn-recent-verb">{e.verb}</span>
              <span class="kn-recent-object">{e.object}</span>
              <span class="kn-recent-time">{clock(e.at, $locale)}</span>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {:else if event}
    <div class="kn-detail-name">{event.verb} {event.object}</div>
    <div class="kn-kv">
      <span class="kn-k">{$t("k.detail.when")}</span>
      <span class="kn-v">{formatModified(event.at)}</span>
    </div>
    <div class="kn-kv">
      <span class="kn-k">{$t("k.detail.source")}</span>
      <span class="kn-v">{event.source}</span>
    </div>
    {#if event.project}
      <div class="kn-kv">
        <span class="kn-k">{$t("k.detail.project")}</span>
        <span class="kn-v">{event.project}</span>
      </div>
    {/if}
    <p class="kn-detail-more">{$t("k.detail.more")}</p>
  {:else if entry}
    <div class="kn-detail-name">{entry.name}</div>

    {#if entry.modified_unix != null}
      <div class="kn-kv">
        <span class="kn-k">{$t("k.detail.when")}</span>
        <span class="kn-v">{formatModified(entry.modified_unix)}</span>
      </div>
    {/if}

    <p class="kn-detail-more">{$t("k.detail.more")}</p>
  {/if}
</aside>

<style>
  .kn-detail {
    flex: 0 0 17rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.75rem 0.9rem;
    border-inline-start: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-y: auto;
  }
  .kn-detail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .kn-detail-title {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
  .kn-detail-close {
    display: inline-flex;
    padding: 0.2rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    cursor: pointer;
  }
  .kn-detail-close:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    color: var(--color-fg-primary);
  }
  .kn-detail-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    line-height: 1.4;
  }
  .kn-kv {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .kn-k {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .kn-v {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
  }
  .kn-detail-more {
    margin: 0.4rem 0 0;
    font-size: var(--text-2xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 48%, transparent);
  }

  /* The project's recent activity: the timeline's sentence anatomy in
     miniature (quiet verb, emphasized object, tabular time). */
  .kn-recent {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin-top: 0.15rem;
  }
  .kn-recent-row {
    display: flex;
    align-items: baseline;
    gap: 0.375rem;
    min-width: 0;
  }
  .kn-recent-verb {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .kn-recent-object {
    flex: 1;
    min-width: 0;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kn-recent-time {
    flex-shrink: 0;
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
