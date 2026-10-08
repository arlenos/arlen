<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->
<!--
  A dev surface for the kit's motion pass (design-system §6b): one row per
  primitive family, at rest, so a gesture can be looked at and frozen.
  `?flip=1` changes every control's state after load; `&at=<ms>` then freezes
  every running transition at that moment, for a proof frame mid-gesture.
  `?reduce=1` zeroes the three themed durations the way the shell's theme does
  under appearance.toml, so the reduced path renders without a shell.
  `?dark=1` paints the shell's ground. `?dialog=1` opens a dialog on the flip.

  Not linked from the demo index, and its labels stay in English: they
  describe the harness, not the product.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { Switch } from "$lib/components/ui/switch";
  import { Checkbox } from "$lib/components/ui/checkbox";
  import { Notice } from "$lib/components/ui/notice";
  import { Button } from "$lib/components/ui/button";
  import { SegmentedControl } from "$lib/components/ui/segmented-control";
  import Dialog from "$lib/components/ui/dialog/dialog.svelte";

  const views = [
    { value: "day", label: "Day" },
    { value: "week", label: "Week" },
    { value: "month", label: "Month" },
    { value: "agenda", label: "Agenda list" },
  ];
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
    CollapsibleChevron,
  } from "$lib/components/ui/collapsible";

  let flipped = $state(false);
  let dark = $state(false);
  let reduce = $state(false);
  let dialog = $state(false);

  onMount(async () => {
    const q = new URLSearchParams(window.location.search);
    dark = q.get("dark") === "1";
    reduce = q.get("reduce") === "1";
    dialog = q.get("dialog") === "1";
    // On the root, where the shell's theme writes them: `--duration-micro` is
    // derived at :root, so a value set lower down would never reach it.
    if (reduce) {
      for (const d of ["fast", "normal", "slow"]) document.documentElement.style.setProperty(`--duration-${d}`, "0ms");
    }
    if (q.get("flip") !== "1") return;
    await new Promise((r) => setTimeout(r, 400));
    flipped = true;
    await tick();
    const at = Number(q.get("at"));
    if (at > 0) {
      // A transition is created on the first style recalc after the change,
      // so the freeze waits one short beat for it to exist.
      setTimeout(() => {
        for (const a of document.getAnimations()) {
          a.pause();
          a.currentTime = at;
        }
      }, 80);
    }
  });
</script>

<div class="page" class:dark>
  <h1>Motion{flipped ? ", flipped" : ""}{reduce ? ", reduced" : ""}</h1>

  <section>
    <h2>Switch</h2>
    <div class="row">
      <Switch value={flipped} ariaLabel="off to on" />
      <Switch value={!flipped} ariaLabel="on to off" />
      <Switch value={flipped} size="sm" ariaLabel="small off to on" />
      <Switch value={!flipped} size="sm" ariaLabel="small on to off" />
      <Switch value={flipped} disabled ariaLabel="disabled" />
    </div>
  </section>

  <section>
    <h2>Checkbox</h2>
    <div class="row">
      <Checkbox checked={flipped} ariaLabel="unticked to ticked" />
      <Checkbox checked={!flipped} ariaLabel="ticked to unticked" />
      <Checkbox checked={flipped} disabled ariaLabel="disabled" />
    </div>
  </section>

  <section>
    <h2>Disclosure</h2>
    <div class="discl">
      <Collapsible open={flipped}>
        <CollapsibleTrigger class="discl-trigger"><CollapsibleChevron />Advanced</CollapsibleTrigger>
        <CollapsibleContent>
          <p>Three lines of settings that sit behind the expander until someone asks for them.</p>
          <p>A second paragraph, so the height is not a single line.</p>
        </CollapsibleContent>
      </Collapsible>
      <Collapsible open={!flipped}>
        <CollapsibleTrigger class="discl-trigger">Earlier message<CollapsibleChevron from="down" /></CollapsibleTrigger>
        <CollapsibleContent>
          <p>The body of a message in a thread, folded away once a later one arrives.</p>
        </CollapsibleContent>
      </Collapsible>
    </div>
  </section>

  <section>
    <h2>Notice</h2>
    <div class="notices">
      {#if flipped}
        <Notice tone="error" text="The file could not be saved, so the copy on disk is the old one." />
        <Notice tone="caution" text="This folder is shared with two other people." />
        <Notice text="Example pictures, not files on this machine." />
      {/if}
    </div>
  </section>

  <section>
    <h2>Button</h2>
    <div class="row">
      <Button id="motion-press">Save</Button>
      <Button variant="outline">Cancel</Button>
      <Button variant="ghost" size="sm">Undo</Button>
    </div>
  </section>

  <section>
    <h2>Tabs</h2>
    <div class="row">
      <SegmentedControl options={views} value={flipped ? "agenda" : "day"} ariaLabel="view" />
      <SegmentedControl options={views} value={flipped ? "day" : "month"} size="compact" ariaLabel="compact view" />
    </div>
  </section>
</div>

<Dialog open={flipped && dialog} onClose={() => (dialog = false)} ariaLabel="example dialog" size="sm">
  <div class="dlg">
    <h3>Delete permanently?</h3>
    <p>This cannot be undone.</p>
  </div>
</Dialog>

<style>
  .page {
    min-height: 100vh;
    padding: 1.5rem;
    color: var(--color-fg-primary);
    background: var(--color-bg-app);
  }
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
  h2 {
    font-size: 0.8rem;
    font-weight: 500;
    opacity: 0.7;
    margin: 0 0 0.6rem;
  }
  section {
    margin-bottom: 1.5rem;
  }
  .discl {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2rem;
    max-width: 40rem;
    align-items: start;
  }
  .discl :global(.discl-trigger) {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font: inherit;
    font-size: 0.85rem;
    background: transparent;
    border: 0;
    color: inherit;
    padding: 0;
  }
  .discl p {
    font-size: 0.8rem;
    opacity: 0.75;
    margin: 0.5rem 0 0;
  }
  .notices {
    display: grid;
    gap: 0.5rem;
    max-width: 30rem;
    min-height: 7.5rem;
  }
  .dlg {
    padding: 1.25rem;
  }
  .dlg h3 {
    margin: 0 0 0.4rem;
    font-size: 0.95rem;
  }
  .dlg p {
    margin: 0;
    font-size: 0.8rem;
    opacity: 0.7;
  }
  .row {
    display: flex;
    gap: 1.5rem;
    align-items: center;
  }
</style>
