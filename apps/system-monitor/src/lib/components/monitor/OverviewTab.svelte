<script lang="ts">
  /// The Overview tab: the sovereign glance. The verdict is the hero (a calm thesis,
  /// "everything's normal, and here's exactly who touched what"), then what's using
  /// sensitive resources right now (the sovereign transparency, each row links to
  /// App-access), then what's using the most (the task-manager utility).
  import { overview, manageAccess, type LiveAccess } from "$lib/stores/overview";

  // "microphone and camera", "camera, microphone and screen".
  function resourcePhrase(resources: string[]): string {
    if (resources.length <= 1) return resources[0] ?? "";
    return `${resources.slice(0, -1).join(", ")} and ${resources[resources.length - 1]}`;
  }
  function memLabel(mb: number): string {
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  }
  function duration(mins: number): string {
    if (mins < 60) return `${mins} min`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m ? `${h} h ${m} min` : `${h} h`;
  }
</script>

<div class="ov">
  <section class="verdict" data-state={$overview.verdict.state}>
    <span class="verdict-dot" aria-hidden="true"></span>
    <div class="verdict-text">
      <h1 class="verdict-headline">{$overview.verdict.headline}</h1>
      <p class="verdict-detail">{$overview.verdict.detail}</p>
    </div>
  </section>

  <section class="block">
    <h2 class="block-title">Right now</h2>
    {#if $overview.liveAccess.length > 0}
      <div class="rows">
        {#each $overview.liveAccess as a (a.appId)}
          <div class="access-row">
            <div class="access-what">
              <span class="access-app">{a.app}</span> is using your {resourcePhrase(a.resources)}
            </div>
            <span class="access-dur">{duration(a.sinceMins)}</span>
            <button type="button" class="access-manage" onclick={() => manageAccess(a.appId)}>
              Manage access
            </button>
          </div>
        {/each}
      </div>
    {:else}
      <p class="block-empty">Nothing is using your camera, microphone, screen, or location.</p>
    {/if}
  </section>

  <section class="block">
    <h2 class="block-title">Using the most</h2>
    <div class="rows">
      {#each $overview.resourceTop as r (r.appId)}
        <div class="use-row">
          <span class="use-app">{r.app}</span>
          <span class="use-cpu">{r.cpu}% CPU</span>
          <span class="use-mem">{memLabel(r.memMB)}</span>
        </div>
      {/each}
    </div>
  </section>
</div>

<style>
  .ov {
    display: flex;
    flex-direction: column;
    gap: 2rem;
    max-width: 46rem;
  }

  /* The verdict is the hero: a calm statement with a single status accent (the dot),
     everything else monochrome. */
  .verdict {
    display: flex;
    align-items: flex-start;
    gap: 0.9rem;
  }
  .verdict-dot {
    flex-shrink: 0;
    width: 0.7rem;
    height: 0.7rem;
    margin-top: 0.5rem;
    border-radius: 999px;
    background: var(--color-success, #4ade80);
  }
  .verdict[data-state="attention"] .verdict-dot {
    background: var(--color-warning, #fbbf24);
  }
  .verdict[data-state="alert"] .verdict-dot {
    background: var(--color-error, #f87171);
  }
  .verdict-headline {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 600;
    line-height: 1.25;
    color: var(--color-fg-primary);
  }
  .verdict-detail {
    margin: 0.3rem 0 0;
    font-size: 0.9375rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .block-title {
    margin: 0 0 0.6rem;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .rows {
    display: flex;
    flex-direction: column;
  }
  .block-empty {
    margin: 0;
    font-size: 0.9375rem;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }

  .access-row {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.7rem 0;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .access-row:first-child {
    border-top: none;
  }
  .access-what {
    flex: 1;
    min-width: 0;
    font-size: 0.9375rem;
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
  }
  .access-app {
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .access-dur {
    font-size: 0.8125rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    white-space: nowrap;
  }
  .access-manage {
    flex-shrink: 0;
    padding: 0.3rem 0.7rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 18%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: 0.8125rem;
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .access-manage:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }

  .use-row {
    display: grid;
    grid-template-columns: 1fr auto auto;
    align-items: center;
    gap: 1.25rem;
    padding: 0.55rem 0;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .use-row:first-child {
    border-top: none;
  }
  .use-app {
    font-size: 0.9375rem;
    color: var(--color-fg-primary);
  }
  .use-cpu,
  .use-mem {
    font-size: 0.8125rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    white-space: nowrap;
  }
  .use-mem {
    min-width: 4.5rem;
    text-align: right;
  }
</style>
