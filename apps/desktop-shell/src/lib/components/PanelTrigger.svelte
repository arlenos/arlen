<script lang="ts">
  import { togglePopover } from "$lib/stores/activePopover.js";
  import { Square } from "lucide-svelte";
</script>

<button
  class="panel-trigger"
  aria-label="Quick Settings"
  onclick={() => togglePopover("quick-settings")}
>
  <Square size={14} strokeWidth={1.5} />
</button>

<style>
  .panel-trigger {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    min-height: 24px;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: 4px;
    cursor: pointer;
    color: var(--foreground);
    transition: background-color var(--duration-fast, 150ms) ease;
  }

  .panel-trigger:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
