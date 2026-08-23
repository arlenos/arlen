<script lang="ts">
  /// App shell: locale and theme come from the system, like every other app.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { t } from "$lib/i18n/messages";
  import { setWindowTitle } from "$lib/window-title";

  let { children } = $props();

  // The topbar and the workspace overview show the NATIVE window title,
  // not the document one below, so it has to be set - and set again when
  // the language changes, which is why this reads `$t` instead of firing
  // once at startup.
  $effect(() => {
    void setWindowTitle($t("wn.app.title"));
  });

  onMount(() => {
    void initArlenLocale();
    void initArlenTheme();
  });
</script>

<svelte:head>
  <title>{$t("wn.app.title")}</title>
</svelte:head>

{@render children()}
