<script lang="ts">
  /// Adopt the chosen language. This app had a catalog and never set the locale,
  /// so it came up in the source language whatever the user chose - the same gap
  /// Settings had, still live here because a layout with nothing else in it is
  /// easy to skip.
  ///
  /// It has no Tauri backend yet, so the read inside fails and today this only
  /// takes effect through the dev `?locale=` override. That is the honest state:
  /// the call is right where it belongs for the day the backend lands, and the
  /// alternative - adding it then - is how it was forgotten the first time.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  // The capability file has granted `theme_get` all along; without this call the
  // app held the permission and stayed in the default palette.
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  let { children } = $props();

  onMount(() => {
    void initArlenLocale();
    void initArlenTheme();
  });
</script>

{@render children()}
