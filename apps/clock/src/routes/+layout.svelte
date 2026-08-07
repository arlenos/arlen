<script lang="ts">
  /// App shell: the compact-utility chrome (titlebar + tabs live in the page).
  /// The layout owns the locale init and the 1 Hz render tick every surface
  /// derives its displays from - the tick renders, the daemon keeps time.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { startTick, loadClock } from "$lib/stores/clock";

  let { children } = $props();

  onMount(() => {
    void initArlenLocale();
    void loadClock();
    return startTick();
  });
</script>

{@render children()}
