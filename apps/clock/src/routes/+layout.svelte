<script lang="ts">
  /// App shell: the compact-utility chrome (titlebar + tabs live in the page).
  /// The layout owns the locale and theme init and the 1 Hz render tick every
  /// surface derives its displays from - the tick renders, the daemon keeps time.
  ///
  /// The theme call was missing while the capability file granted it, so this app
  /// held a permission it never used and sat in the default palette while every
  /// other app followed the system.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { startTick, loadClock } from "$lib/stores/clock";

  let { children } = $props();

  onMount(() => {
    void initArlenLocale();
    void initArlenTheme();
    void loadClock();
    return startTick();
  });
</script>

{@render children()}
