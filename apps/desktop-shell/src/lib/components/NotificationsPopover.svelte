<script lang="ts">
  /// Notifications popover.
  ///
  /// Standalone surface — separate from Quick Settings since
  /// notifications need a focused, fast-access channel that doesn't
  /// fight with controls. Anchored top-right under the
  /// `NotificationsTrigger` bell icon, sharing the QS anchor + width
  /// so the right column reads as one unified surface across QS /
  /// Notifications / Toasts (mutually exclusive via `activePopover`).
  ///
  /// `keepMounted`: NotificationPanel keeps its grouped-by-app
  /// subscriptions alive across opens, so opening the panel doesn't
  /// re-fetch history.
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import NotificationPanel from "$lib/components/NotificationPanel.svelte";
</script>

<ShellPopover
  id="notifications"
  width={380}
  right={8}
  maxHeight="calc(100vh - 56px)"
  keepMounted
  bodyPadding="12px"
>
  <NotificationPanel />
</ShellPopover>
