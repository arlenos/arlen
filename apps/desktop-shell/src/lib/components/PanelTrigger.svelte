<script lang="ts">
  import { togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import { unreadCount } from "$lib/stores/notifications.js";
  import { Square } from "lucide-svelte";

  const badgeText = $derived(
    $unreadCount > 99 ? "99+" : $unreadCount > 0 ? String($unreadCount) : ""
  );
</script>

<button
  class="panel-trigger"
  aria-label="Quick Settings"
  onclick={() => togglePopover("quick-settings")}
  onmouseenter={() => hoverPopover("quick-settings")}
>
  <Square size={14} strokeWidth={1.5} />
  {#if badgeText}
    <span class="panel-badge">{badgeText}</span>
  {/if}
</button>

<style>
  .panel-trigger {
    position: relative;
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
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--foreground);
    transition:
      transform var(--duration-micro) var(--ease-out),
      background-color var(--duration-fast) var(--ease-out);
  }

  .panel-trigger:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .panel-trigger:active {
    transform: scale(0.96);
  }

  .panel-badge {
    position: absolute;
    top: 1px;
    right: 0px;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: var(--radius-md);
    background: var(--color-error);
    color: var(--color-fg-inverse);
    font-size: 0.5625rem;
    font-weight: 700;
    line-height: 14px;
    text-align: center;
    pointer-events: none;
    animation: lunaris-badge-in var(--duration-fast) var(--ease-bounce);
  }
</style>
