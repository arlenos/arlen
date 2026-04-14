<script lang="ts">
  /// Notification bell indicator for the top bar.
  ///
  /// Shows unread count badge. Bell icon when normal, BellOff when DND.

  import { unreadCount, dndState } from "$lib/stores/notifications.js";
  import { togglePopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Bell, BellOff } from "lucide-svelte";

  const tooltipClass =
    "rounded-md border px-2 py-0.5 text-xs shadow-md select-none"
    + " bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  const Icon = $derived($dndState.mode !== "off" ? BellOff : Bell);

  const label = $derived(
    $dndState.mode !== "off"
      ? "Notifications: Do Not Disturb"
      : $unreadCount > 0
        ? `Notifications: ${$unreadCount} unread`
        : "Notifications"
  );

  const badgeText = $derived(
    $unreadCount > 99 ? "99+" : $unreadCount > 0 ? String($unreadCount) : ""
  );
</script>

<Tooltip.Root>
  <Tooltip.Trigger>
    {#snippet child({ props })}
      <button
        class="notif-btn"
        class:dnd={$dndState.mode !== "off"}
        aria-label={label}
        {...props}
        onclick={() => togglePopover("quick-settings")}
      >
        <Icon size={14} strokeWidth={1.5} />
        {#if badgeText}
          <span class="notif-badge">{badgeText}</span>
        {/if}
      </button>
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Portal>
    <Tooltip.Content side="bottom" class={tooltipClass}>{label}</Tooltip.Content>
  </Tooltip.Portal>
</Tooltip.Root>

<style>
  .notif-btn {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--foreground);
    transition: background-color 150ms ease;
  }
  .notif-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .notif-btn.dnd {
    opacity: 0.5;
  }
  .notif-badge {
    position: absolute;
    top: 2px;
    right: 1px;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: var(--radius-md);
    background: #ef4444;
    color: #fff;
    font-size: 0.5625rem;
    font-weight: 700;
    line-height: 14px;
    text-align: center;
    pointer-events: none;
  }
</style>
