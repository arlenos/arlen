<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->
<!--
  A dev surface for looking at translated kit chrome: `just kit-dev`, then
  /_i18n?locale=de.

  The catalog gate proves every message compiles and the lint proves no string is
  hardcoded, but neither can see that a German label is now too wide for its pill or
  that a locale-derived weekday came out in the wrong order. Those are pixel facts,
  and this is where to read them - one screenshot per locale, diffed by eye.

  One locale per load rather than all four down the page: `locale` is a single global
  store, so a row cannot hold its own. The first cut of this page set it per row
  during render, which invalidated the translator mid-render and painted nothing at
  all - a blank screenshot, which is exactly the failure a screenshot loop exists to
  catch.

  Not linked from the demo index, and its own labels stay in English: they describe
  the harness, not the product.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { locale, Rich, mark } from "$lib/i18n";
  import { DaysPicker } from "$lib/components/ui/days-picker";
  import { ConsoleBlock } from "$lib/components/console";
  import { AboutDialog } from "$lib/components/ui/about-dialog";

  let days = $state([0, 2, 4]);
  let aboutOpen = $state(true);
  let active = $state("en");

  onMount(() => {
    // `?locale=` also accepts a tag the kit catalog does not ship (try `fr`): the
    // text falls back to English while dates and numbers still come from that
    // locale's own CLDR data, and seeing the two halves disagree is the point.
    const want = new URLSearchParams(window.location.search).get("locale");
    if (want) {
      active = want;
      locale.set(want);
    }
  });
</script>

<div class="page">
  <h1>Kit chrome &mdash; locale {active}</h1>

  <section>
    <h2>days picker</h2>
    <DaysPicker value={days} onchange={(v) => (days = v)} />
  </section>

  <section>
    <h2>console block</h2>
    <ConsoleBlock command="cargo build" exitCode={1} durationMs={2400} />
    <ConsoleBlock command="cargo test" running={true} />
  </section>

  <section>
    <h2>rich sentence</h2>
    <!-- `Rich` is unit-tested on its part splitting but had never rendered in a
         browser, and a snippet passed as a rest prop is exactly the kind of thing
         that typechecks and then does not run. -->
    <p>
      <Rich text={"Run " + mark("cmd") + " or drop a file into " + mark("dir") + "."}>
        {#snippet cmd()}<code>arlen install</code>{/snippet}
        {#snippet dir()}<code>~/.local/share/arlen</code>{/snippet}
      </Rich>
    </p>
    <p>
      <!-- A name with no snippet must render the name, not vanish. -->
      <Rich text={"missing snippet renders as " + mark("absent")} />
    </p>
  </section>

  <section>
    <h2>about dialog</h2>
    <AboutDialog
      open={aboutOpen}
      onClose={() => (aboutOpen = false)}
      appName="Files"
      version="0.1.0"
      description="Browse and organise your files."
    />
  </section>
</div>

<style>
  .page {
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  h1 {
    font-size: var(--text-lg);
    font-weight: 600;
  }
  h2 {
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    opacity: 0.5;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
</style>
