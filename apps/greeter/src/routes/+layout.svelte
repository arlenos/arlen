<script lang="ts">
  /// Root layout: the greeter is a single fullscreen surface that runs
  /// before any session. No sidebar, no window controls (there is no
  /// window to manage and nothing to minimize to). It only loads the
  /// tokens and keeps the accessibility options reflected onto the root.
  import "../app.css";
  import { a11y, applyA11y } from "$lib/a11y";

  let { children } = $props();

  // Reflect contrast + text-scale onto the document root whenever they
  // change, so the CSS variables take effect immediately.
  $effect(() => applyA11y($a11y));
</script>

<div class="shell">
  {@render children?.()}
</div>

<style>
  .shell {
    position: relative;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
  }
</style>
