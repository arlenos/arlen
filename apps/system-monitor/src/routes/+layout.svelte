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
    void setWindowTitle($t("tm.app.title"));
  });

  onMount(() => {
    // The chosen language and the live theme. This app embeds the shell plugin
    // and has the permission; it simply never asked, so it kept the defaults
    // whatever the user picked.
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
  <title>{$t("tm.app.title")}</title>
</svelte:head>

{@render children()}
