<script lang="ts">
  /// The Activity/Jobs zone (job-progress-surface.md): the top zone of the
  /// notifications popover, showing live long-running work with a per-job progress
  /// bar, real-unit counts, a coarse ETA, cancel/pause driven by capability flags,
  /// and an expandable per-item list (never hide the per-file names). Running jobs
  /// sit above done receipts. Self-guards when there is no job. Fixture-backed.
  import { onMount } from "svelte";
  import Progress from "@arlen/ui-kit/components/ui/progress/progress.svelte";
  import { Pause, Play, X, RotateCw, ChevronRight, Check } from "lucide-svelte";
  import {
    jobs,
    pollJobs,
    cancelJob,
    pauseJob,
    resumeJob,
    type Job,
  } from "$lib/stores/jobs";

  onMount(() => {
    void pollJobs();
  });

  // Running/active first, done receipts last.
  const ordered = $derived(
    [...$jobs].sort((a, b) => (a.state === "done" ? 1 : 0) - (b.state === "done" ? 1 : 0)),
  );

  let expanded = $state<Set<string>>(new Set());
  function toggle(id: string) {
    const next = new Set(expanded);
    next.has(id) ? next.delete(id) : next.add(id);
    expanded = next;
  }

  const isError = (j: Job) => j.state === "error_recoverable" || j.state === "error_fatal";
  function avatarLetter(label: string) {
    return label.charAt(0).toUpperCase();
  }
</script>

{#if ordered.length > 0}
  <div class="jobs-zone">
    <div class="jobs-head">
      <span class="jobs-title">Activity</span>
      <span class="jobs-count">{ordered.length}</span>
    </div>

    {#each ordered as j (j.id)}
      <div class="job" class:paused={j.state === "paused"} class:err={isError(j)} class:done={j.state === "done"}>
        <div class="job-top">
          <span class="job-avatar">{avatarLetter(j.appLabel)}</span>
          <span class="job-title">{j.title}</span>
          <span class="job-actions">
            {#if j.suspendable && j.state === "running"}
              <button class="job-btn" aria-label="Pause" title="Pause" onclick={() => pauseJob(j.id)}><Pause size={14} strokeWidth={2} /></button>
            {:else if j.suspendable && j.state === "paused"}
              <button class="job-btn" aria-label="Resume" title="Resume" onclick={() => resumeJob(j.id)}><Play size={14} strokeWidth={2} /></button>
            {/if}
            {#if j.state === "error_recoverable"}
              <button class="job-btn" aria-label="Retry" title="Retry" onclick={() => resumeJob(j.id)}><RotateCw size={14} strokeWidth={2} /></button>
            {/if}
            {#if j.killable && j.state !== "done"}
              <button class="job-btn" aria-label="Cancel" title="Cancel" onclick={() => cancelJob(j.id)}><X size={14} strokeWidth={2} /></button>
            {/if}
          </span>
        </div>

        {#if j.state === "done"}
          <div class="job-doneline"><Check size={13} strokeWidth={2} /> Done</div>
        {:else}
          <Progress value={j.fraction * 100} />
          <div class="job-meta">
            <span class="job-metrics">
              {#each j.metrics as m (m.unit)}
                <span>{m.processed} of {m.total} {m.unit}</span>
              {/each}
            </span>
            {#if j.state === "running" && j.etaText}
              <span class="job-eta">{j.etaText} left</span>
            {:else if j.state === "paused"}
              <span class="job-eta">Paused</span>
            {:else if j.state === "impeded"}
              <span class="job-eta">Waiting</span>
            {/if}
          </div>
        {/if}

        {#if j.egressHost}
          <div class="job-egress">Reaches {j.egressHost}</div>
        {/if}
        {#if isError(j) && j.error}
          <div class="job-error">{j.error}</div>
        {/if}

        {#if j.items && j.items.length > 0}
          <button class="job-expand" aria-expanded={expanded.has(j.id)} onclick={() => toggle(j.id)}>
            <ChevronRight size={13} strokeWidth={2} class={expanded.has(j.id) ? "rot" : ""} />
            {expanded.has(j.id) ? "Hide items" : `${j.items.length} items`}
          </button>
          {#if expanded.has(j.id)}
            <div class="job-items">
              {#each j.items as it (it.name)}
                <div class="job-item" class:item-done={it.done}>
                  <span class="job-item-dot" class:on={it.done}></span>
                  <span class="job-item-name">{it.name}</span>
                </div>
              {/each}
            </div>
          {/if}
        {/if}
      </div>
    {/each}

    <div class="jobs-divider"></div>
  </div>
{/if}

<style>
  .jobs-zone {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  .jobs-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0 0.125rem;
  }
  .jobs-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .jobs-count {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  .job {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.625rem;
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .job-top {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .job-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .job-title {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .job-actions {
    display: inline-flex;
    gap: 0.125rem;
    flex-shrink: 0;
  }
  .job-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border: none;
    background: transparent;
    border-radius: var(--radius-input);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .job-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }

  /* State tints on the bar fill, without forking the Progress primitive. */
  .job.paused :global(.progress-fill) {
    background: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .job.err :global(.progress-fill) {
    background: var(--color-error);
  }

  .job-meta {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .job-metrics {
    display: inline-flex;
    gap: 0.75rem;
    min-width: 0;
    font-variant-numeric: tabular-nums;
  }
  .job-eta {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .job-egress {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .job-error {
    font-size: 0.6875rem;
    line-height: 1.4;
    color: var(--color-error);
  }
  .job-doneline {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.75rem;
    color: var(--color-success);
  }

  .job-expand {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    align-self: flex-start;
    padding: 0.125rem 0;
    border: none;
    background: transparent;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
  }
  .job-expand:hover {
    color: var(--foreground);
  }
  .job-expand :global(.rot) {
    transform: rotate(90deg);
  }
  .job-items {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding-left: 0.25rem;
  }
  .job-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .job-item-dot {
    width: 6px;
    height: 6px;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    border: 1px solid color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .job-item-dot.on {
    background: var(--color-success);
    border-color: var(--color-success);
  }
  .job-item-name {
    font-family: var(--font-mono, ui-monospace, monospace);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-done .job-item-name {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  .jobs-divider {
    height: 1px;
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    margin-top: 0.125rem;
  }
</style>
