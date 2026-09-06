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

<main class="hint">Print dialog look-mock. The dialog opens over this page.</main>

<style>
  /* A ground and a legible hint. This page had neither, so its one line sat at
     1.01:1 on the browser's white default - the same shape as `_toasttest`, found
     the same way, and painted on the element rather than on `body` for the same
     reason: the shell's body is transparent on purpose. */
  .hint {
    min-height: 100vh;
    background: var(--color-bg-shell, #0a0a0a);
    padding: 2rem;
    font-size: 0.8rem;
    color: color-mix(in srgb, var(--foreground, #fff) 55%, transparent);
  }
</style>
