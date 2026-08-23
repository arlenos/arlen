<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";

  let { children } = $props();

  // The topbar and the workspace overview show the NATIVE window title,
  // not the document one below, so it has to be set - and set again when
  // the language changes, which is why this reads `$t` instead of firing
  // once at startup.
  $effect(() => {
    void setWindowTitle($t("s.app.title"));
  });

  onMount(() => {
    // The chosen language and the live theme, before anything renders in the
    // wrong one.
    void initArlenLocale();
    void initArlenTheme();
  });
  import { t } from "$lib/i18n/messages";
  import { setWindowTitle } from "$lib/window-title";
</script>

<svelte:head>
  <!-- The document title: what a screen reader announces for the window
       and what a task switcher shows. Every Arlen app was missing one,
       which axe reports as `document-title` on every surface. -->
  <title>{$t("s.app.title")}</title>
</svelte:head>

{@render children?.()}
