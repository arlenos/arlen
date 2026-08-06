<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";

  let { children } = $props();

  onMount(() => {
    // The chosen language and the live theme. This app embeds the shell plugin
    // and has the permission; it simply never asked, so it kept the defaults
    // whatever the user picked.
    void initArlenLocale();
    void initArlenTheme();
  });
</script>

{@render children()}
