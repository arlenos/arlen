<script lang="ts">
  /// Dev/test route for the print dialog (printing-plan.md PRN-R3). The shell layout
  /// mounts `PrintDialog`; here we open its fixture so the surface can be viewed +
  /// audited under vite, on a topbar-free page (the root route mounts the topbar,
  /// whose Tauri-only calls do not run under vite). Live, the portal print event
  /// drives openPrintDialog instead. Not shipped in any nav, like `_mpristest`.
  import { onMount } from "svelte";
  import { openPrintDialog } from "$lib/stores/printDialog";

  onMount(() => {
    void openPrintDialog();
  });
</script>

<div class="hint">Print dialog look-mock. The dialog opens over this page.</div>

<style>
  .hint {
    padding: 2rem;
    font-size: 0.8rem;
    color: color-mix(in srgb, var(--foreground, #fff) 40%, transparent);
  }
</style>
