<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";

  let { children } = $props();

  onMount(() => {
    // The chosen language and the live theme, before anything renders in the
    // wrong one.
    void initArlenLocale();
    void initArlenTheme();
  });
  import { t } from "$lib/i18n/messages";
</script>

<svelte:head>
  <!-- The document title: what a screen reader announces for the window
       and what a task switcher shows. Every Arlen app was missing one,
       which axe reports as `document-title` on every surface. -->
  <title>{$t("s.app.title")}</title>
</svelte:head>

{@render children?.()}
