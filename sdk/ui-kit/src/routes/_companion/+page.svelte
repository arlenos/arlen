<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->
<!--
  A dev surface for looking at the companion: `just kit-dev`, then
  /_companion?state=done&talking=1&bust=1&radius=50%25&reduce=1.

  The unit test proves the contract under jsdom, which cannot draw; this page
  is where the morphs, the accents, the talking mouth and the reduce-motion
  jump are looked at, one screenshot per query. `reduce=1` sets
  `--duration-normal: 0ms` on the page the way the shell's theme does under
  appearance.toml, so the reduce-motion path renders without a shell.

  Not linked from the demo index, and its labels stay in English: they
  describe the harness, not the product.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { Companion, COMPANION_STATES, type CompanionState } from "$lib/components/ui/companion";

  let current = $state<CompanionState>("resting");
  let talking = $state(false);
  let radius = $state("var(--radius-card)");
  let reduce = $state(false);

  onMount(() => {
    const q = new URLSearchParams(window.location.search);
    const want = q.get("state") as CompanionState | null;
    if (want && COMPANION_STATES.includes(want)) current = want;
    talking = q.get("talking") === "1";
    if (q.get("radius")) radius = q.get("radius") as string;
    reduce = q.get("reduce") === "1";
  });
</script>

<div class="page" style={reduce ? "--duration-normal: 0ms" : ""}>
  <h1>Companion &mdash; {current}{talking ? ", talking" : ""}{reduce ? ", reduced motion" : ""}</h1>

  <div class="bar">
    {#each COMPANION_STATES as s (s)}
      <button type="button" class:on={current === s} onclick={() => (current = s)}>{s}</button>
    {/each}
    <label><input type="checkbox" bind:checked={talking} /> talking</label>
    <label><input type="checkbox" bind:checked={reduce} /> reduce motion</label>
  </div>

  <div class="row">
    <div class="cell"><Companion state={current} {talking} size={192} /><small>192</small></div>
    <div class="cell"><Companion state={current} {talking} size={40} /><small>40, beside a text box</small></div>
    <div class="cell"><Companion state={current} {talking} size={96} bust {radius} /><small>bust 96</small></div>
    <div class="cell"><Companion state={current} {talking} size={40} bust {radius} /><small>bust 40</small></div>
    <div class="cell"><Companion state={current} {talking} size={40} bust radius="50%" /><small>bust 40, circle</small></div>
  </div>
</div>

<style>
  .page {
    padding: 1.5rem;
    color: var(--color-fg-primary);
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
    gap: 2rem;
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.4rem;
  }
  small {
    font-size: 0.7rem;
    opacity: 0.6;
  }
</style>
