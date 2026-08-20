<script lang="ts">
  /// App shell: locale and theme come from the system, like every other app.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { t } from "$lib/i18n/messages";

  let { children } = $props();

  onMount(() => {
    void initArlenLocale();
    void initArlenTheme();
  });
</script>

<svelte:head>
  <title>{$t("ml.app.title")}</title>
</svelte:head>

{@render children()}
