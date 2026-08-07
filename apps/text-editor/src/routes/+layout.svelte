<script lang="ts">
  /// Adopt the chosen language before anything renders in the wrong one. This
  /// app had a catalog and never set the locale, so it came up in the source
  /// language whatever the user chose - the same gap Settings had, still live
  /// here because a layout with nothing else in it is easy to skip.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  let { children } = $props();

  onMount(() => {
    void initArlenLocale();
  });
</script>

{@render children()}
