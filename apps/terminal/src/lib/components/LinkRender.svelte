<script lang="ts">
  /// A link from command output: looks like text, becomes clickable
  /// on hover, and always shows its real target — never a styled
  /// button (terminal.md §4.8/§6). Only http(s) targets are
  /// clickable at all; anything else renders as inert text.
  import { invoke } from "@tauri-apps/api/core";

  let { url, text }: { url: string; text: string } = $props();

  const allowed = $derived(/^https?:\/\//.test(url));

  async function open() {
    if (!allowed) return;
    try {
      await invoke("open_url", { url });
    } catch {
      // No opener registered (mock, or the command is not wired
      // yet) — the link stays inert rather than failing loudly.
    }
  }
</script>

{#if allowed}
  <button class="link-render" onclick={open}>
    <span class="lr-text">{text}</span>
    <span class="lr-target">{url}</span>
  </button>
{:else}
  <span class="link-render-inert">{text} ({url})</span>
{/if}

<style>
  .link-render {
    display: inline-flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 0;
    border: none;
    background: transparent;
    text-align: left;
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .lr-text {
    font-size: var(--text-sm);
    color: var(--foreground);
    border-bottom: 1px solid transparent;
    transition: border-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .link-render:hover .lr-text {
    border-bottom-color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  .lr-target {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 60ch;
  }

  .link-render-inert {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
