<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->
<!--
  A dev surface for looking at the app icons: `vite dev`, then
  /_icons?state=hover&dark=1&reduce=1&at=200.

  The unit test proves the contract under jsdom, which cannot draw; this page
  is where the glyphs are read at the taskbar's 24 and the launcher's 48, and
  where the gestures are looked at. `state` puts every icon in that state on
  load; without it the icons rest, and the pointer drives them: over is hover,
  a click is open. `at=<ms>` freezes every animation at that moment for a
  proof frame. `dark=1` paints the page the way the shell does, `reduce=1`
  sets `--duration-normal: 0ms` the way the shell's theme does under
  appearance.toml.

  Not linked from the demo index, and its labels stay in English: they
  describe the harness, not the product.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { AppIcon, APP_IDS, APP_ICON_STATES, type AppIconState, type AppId } from "$lib/components/ui/app-icon";

  const SIZES = [24, 32, 48, 64];
  let forced = $state<AppIconState | null>(null);
  let dark = $state(false);
  let reduce = $state(false);
  let states = $state<Record<string, AppIconState>>({});

  onMount(() => {
    const q = new URLSearchParams(window.location.search);
    const want = q.get("state") as AppIconState | null;
    if (want && APP_ICON_STATES.includes(want)) forced = want;
    dark = q.get("dark") === "1";
    reduce = q.get("reduce") === "1";
    // `at=<ms>` freezes every animation at that moment, for a proof frame.
    const at = Number(q.get("at"));
    if (at > 0) {
      setTimeout(() => {
        for (const a of document.getAnimations()) {
          a.pause();
          a.currentTime = at;
        }
      }, 200);
    }
  });

  function key(app: AppId, size: number): string {
    return `${app}@${size}`;
  }
  function stateOf(app: AppId, size: number): AppIconState {
    return forced ?? states[key(app, size)] ?? "rest";
  }
  function set(app: AppId, size: number, s: AppIconState): void {
    states[key(app, size)] = s;
  }
</script>

<div class="page" class:dark style={reduce ? "--duration-normal: 0ms" : ""}>
  <h1>App icons{forced ? `, all ${forced}` : ""}{dark ? ", dark" : ""}{reduce ? ", reduced motion" : ""}</h1>

  <div class="bar">
    {#each APP_ICON_STATES as s (s)}
      <button type="button" class:on={forced === s} onclick={() => (forced = forced === s ? null : s)}>{s}</button>
    {/each}
    <label><input type="checkbox" bind:checked={dark} /> dark</label>
    <label><input type="checkbox" bind:checked={reduce} /> reduce motion</label>
  </div>

  {#each SIZES as size (size)}
    <div class="row">
      <small class="size">{size}</small>
      {#each APP_IDS as app (app)}
        <button
          type="button"
          class="cell"
          onpointerenter={() => set(app, size, "hover")}
          onpointerleave={() => set(app, size, "rest")}
          onclick={() => set(app, size, "open")}
        >
          <AppIcon {app} {size} state={stateOf(app, size)} />
          {#if size === 64}<small>{app}</small>{/if}
        </button>
      {/each}
    </div>
  {/each}
</div>

<style>
  .page {
    min-height: 100vh;
    padding: 1.5rem;
    color: var(--color-fg-primary);
    background: var(--color-bg-app);
  }
  /* the shell's own ground, for looking at the set the way the launcher shows it */
  .page.dark {
    --color-bg-app: #141416;
    --color-fg-primary: #e8e8ec;
    --foreground: #e8e8ec;
    --color-border: #2a2a2e;
  }
  h1 {
    font-size: 0.95rem;
    margin: 0 0 1rem;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
    margin-bottom: 1.25rem;
  }
  button {
    font: inherit;
    font-size: 0.8rem;
    padding: 0.25rem 0.6rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-button);
    background: transparent;
    color: inherit;
  }
  button.on {
    background: var(--color-fg-primary);
    color: var(--color-bg-app);
  }
  label {
    font-size: 0.8rem;
    margin-inline-start: 0.5rem;
  }
  .row {
    display: flex;
    gap: 1.25rem;
    align-items: flex-start;
    flex-wrap: wrap;
    margin-bottom: 1.5rem;
  }
  .size {
    width: 2rem;
    align-self: center;
  }
  .cell {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.4rem;
    min-width: 64px;
  }
  small {
    font-size: 0.7rem;
    opacity: 0.6;
  }
</style>
