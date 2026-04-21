<script lang="ts">
  /// Icon rendered inside a svelte-sonner toast via the per-toast `icon`
  /// prop. Mirrors the panel's NotificationItem icon: `<img>` when a
  /// data URL is available (populated by the Rust resolver in
  /// `notifications/client.rs::resolve_icon`), letter fallback otherwise.
  ///
  /// svelte-sonner instantiates `<toast.icon />` without props, so the
  /// caller in `notifications.ts::fireToast` wraps this component in a
  /// closure that bakes `iconUrl` and `appName` in as pre-bound props.

  let {
    iconUrl = "",
    appName = "",
  }: {
    iconUrl?: string;
    appName?: string;
  } = $props();

  const letter = $derived(appName ? appName.charAt(0).toUpperCase() : "?");
</script>

{#if iconUrl}
  <img src={iconUrl} alt="" class="toast-icon-img" />
{:else}
  <span class="toast-icon-letter" aria-hidden="true">{letter}</span>
{/if}

<style>
  .toast-icon-img {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    object-fit: contain;
    flex-shrink: 0;
  }

  .toast-icon-letter {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, currentColor 12%, transparent);
    color: inherit;
    font-size: 0.75rem;
    font-weight: 600;
    line-height: 1;
    flex-shrink: 0;
    opacity: 0.7;
  }
</style>
