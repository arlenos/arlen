<script lang="ts">
  /// Root layout: the greeter is a single fullscreen surface that runs
  /// before any session. No sidebar, no window controls (there is no
  /// window to manage and nothing to minimize to). It only loads the
  /// tokens and keeps the accessibility options reflected onto the root.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale, locale } from "@arlen/ui-kit/i18n";
  import { a11y, applyA11y } from "$lib/a11y";

  let { children } = $props();

  // Every other app calls this at startup; the greeter did not, so its locale
  // store kept the source language and every German string in its catalogue was
  // unreachable - on the one screen a first-run reader has nothing else to judge
  // the system by. Rendered with `?locale=de` it came up in English, clock and
  // all, which is how this was found.
  //
  // OPEN, and the planner's to settle: WHICH language a login screen should
  // speak. The command this reads (`locale_get`) is the signed-in user's choice,
  // and the greeter runs before login as its own uid, so it resolves to nothing
  // and the source language stands - no worse than today. The candidates are the
  // system default from /etc/locale.conf until a profile is picked, then that
  // profile's own choice once one is; the second half needs the greeter to read
  // another user's config, which is a permissions question, not a wiring one.
  onMount(() => {
    void initArlenLocale().then(() => {
      // Nothing answers `locale_get` here, so without this the store keeps the
      // source language while the clock beside it formats with the environment's
      // - two languages on one screen, which the first German render showed as a
      // German sentence under an English date. The environment IS the system
      // default the greeter should speak until a profile is picked, so adopt it
      // for both. Skipped when a dev session forced one, so `?locale=` still wins.
      const forced = new URLSearchParams(location.search).get("locale");
      if (!forced && navigator.language) locale.set(navigator.language);
    });
  });

  // Reflect contrast + text-scale onto the document root whenever they
  // change, so the CSS variables take effect immediately.
  $effect(() => applyA11y($a11y));
  import { t } from "$lib/i18n/messages";
</script>

<svelte:head>
  <!-- The document title: what a screen reader announces for the window
       and what a task switcher shows. Every Arlen app was missing one,
       which axe reports as `document-title` on every surface. -->
  <title>{$t("g.app.title")}</title>
</svelte:head>

<div class="shell">
  {@render children?.()}
</div>

<style>
  .shell {
    position: relative;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
  }
</style>
