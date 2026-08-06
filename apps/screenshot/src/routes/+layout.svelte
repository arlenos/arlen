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
</script>

{@render children?.()}
