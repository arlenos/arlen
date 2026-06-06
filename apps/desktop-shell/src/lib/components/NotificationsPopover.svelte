<script lang="ts">
  /// Notifications popover.
  ///
  /// Standalone surface — separate from Quick Settings since
  /// notifications need a focused, fast-access channel that doesn't
  /// fight with controls. Anchored top-right under the
  /// `NotificationsTrigger` bell icon.
  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import NotificationPanel from "$lib/components/NotificationPanel.svelte";

  function handleKeydown(e: KeyboardEvent) {
    let current: string | null = null;
    activePopover.subscribe((v) => (current = v))();
    if (current !== "notifications") return;
    if (e.key === "Escape") {
      e.preventDefault();
      closePopover();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- Backdrop only mounts while open — light DOM, no state. -->
{#if $activePopover === "notifications"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="np-backdrop" onclick={() => closePopover()}></div>
{/if}

<!-- Popover is permanently mounted; visibility toggles via class.
     NotificationPanel keeps its grouped-by-app subscriptions alive
     across opens, so opening the panel doesn't re-fetch history. -->
<div
  class="np-popover shell-popover"
  class:visible={$activePopover === "notifications"}
>
  <NotificationPanel />
</div>

<style>
  .np-backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
  }
  .np-popover {
    /* Same anchor + width as QuickSettingsPanel so the right column
       reads as a single unified surface across QS / Notifications /
       Toasts. Mutually exclusive with QS via the `activePopover`
       store, so they never overlap visually. Permanently mounted
       to keep notification listeners alive across open/close. */
    position: fixed;
    top: 40px;
    right: 8px;
    z-index: 100;
    width: 380px;
    max-height: calc(100vh - 56px);
    overflow-y: auto;
    border-radius: var(--radius-card);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    padding: 12px;
    transform-origin: top center;
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    transform: translateY(-4px) scale(0.98);
    transition:
      opacity var(--duration-medium) var(--ease-out),
      transform var(--duration-medium) var(--ease-out),
      visibility 0s linear var(--duration-medium);
  }
  .np-popover.visible {
    opacity: 1;
    visibility: visible;
    pointer-events: auto;
    transform: translateY(0) scale(1);
    transition:
      opacity var(--duration-medium) var(--ease-out),
      transform var(--duration-medium) var(--ease-out),
      visibility 0s linear 0s;
  }
</style>
