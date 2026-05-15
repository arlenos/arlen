<script lang="ts">
  /// Top-bar trigger for the Notifications popover.
  ///
  /// Wraps the shared `Applet` primitive — the icon, hover-state,
  /// hit-target, and tooltip are all shell-controlled. This file
  /// only owns the unread-count → badge wiring + popover toggle
  /// glue.
  ///
  /// Always-visible (matches system-indicator pattern). The bell
  /// `dimmed`s to ~40% when there are zero unread notifications so
  /// the trigger reads as "calm"; full opacity + accent badge when
  /// unread > 0.
  import {
    activePopover,
    togglePopover,
    hoverPopover,
  } from "$lib/stores/activePopover.js";
  import { unreadCount } from "$lib/stores/notifications.js";
  import { Applet, AppletBadge } from "@lunaris/ui-kit/components/topbar";
  import { Bell, BellRing } from "lucide-svelte";

  const hasUnread = $derived($unreadCount > 0);
  const isOpen = $derived($activePopover === "notifications");
</script>

<Applet
  appletId="notifications"
  tooltip={hasUnread
    ? `Notifications — ${$unreadCount} unread`
    : "Notifications"}
  ariaLabel={hasUnread
    ? `${$unreadCount} unread notifications`
    : "Notifications"}
  popoverOpen={isOpen}
  dimmed={!hasUnread && !isOpen}
  onclick={() => togglePopover("notifications")}
  onmouseenter={() => hoverPopover("notifications")}
>
  {#snippet icon()}
    {#if isOpen}
      <BellRing size={14} strokeWidth={1.75} />
    {:else}
      <Bell size={14} strokeWidth={1.75} />
    {/if}
  {/snippet}
  {#snippet badge()}
    <AppletBadge variant="count" value={$unreadCount} color="accent" />
  {/snippet}
</Applet>
