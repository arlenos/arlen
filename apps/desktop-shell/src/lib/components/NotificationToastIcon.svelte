<script lang="ts">
  /// Icon rendered inside a svelte-sonner toast via the per-toast `icon`
  /// prop. Mirrors the panel's NotificationItem icon: `<img>` when a
  /// data URL is available (populated by the Rust resolver in
  /// `notifications/client.rs::resolve_icon`), letter fallback otherwise.
  ///
  /// svelte-sonner instantiates `<toast.icon />` without props, so the
  /// caller in `notifications.ts::fireToast` wraps this component in a
  /// closure that bakes `iconUrl` and `appName` in as pre-bound props.

  import { AppIcon, type AppId } from "@arlen/ui-kit/components/ui/app-icon";

  let {
    iconUrl = "",
    appName = "",
    appId = null,
  }: {
    iconUrl?: string;
    appName?: string;
    /// One of OUR apps, when the notification came from one. It draws the kit's
    /// icon in its `notify` state - the one-off rock the component owns - instead
    /// of the raster the daemon resolved. Null for every other app, and the
    /// `<img>` and letter fallbacks below carry those exactly as before.
    appId?: AppId | null;
  } = $props();

  const letter = $derived(appName ? appName.charAt(0).toUpperCase() : "?");
</script>

{#if appId}
  <AppIcon app={appId} state="notify" size={26} />
{:else if iconUrl}
  <img src={iconUrl} alt="" class="toast-icon-img" />
{:else}
  <span class="toast-icon-letter" aria-hidden="true">{letter}</span>
{/if}

<style>
  .toast-icon-img {
    width: var(--height-control-compact, 26px);
    height: var(--height-control-compact, 26px);
    border-radius: var(--radius-chip);
    object-fit: contain;
    flex-shrink: 0;
  }

  .toast-icon-letter {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 26px);
    height: var(--height-control-compact, 26px);
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, currentColor 12%, transparent);
    color: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    line-height: 1;
    flex-shrink: 0;
    opacity: 0.7;
  }
</style>
