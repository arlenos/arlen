<script lang="ts">
  /// Single notification item card.
  ///
  /// macOS-style notification card: subtle filled bg + border so the
  /// item reads as its own surface, app-icon on the left, content
  /// column with title/body/actions, time + dismiss in the top-right
  /// corner. Dismiss is always visible (no hover-to-reveal) so users
  /// can clear notifications without first having to discover the
  /// affordance.
  import { X } from "lucide-svelte";
  import {
    dismissNotification,
    invokeAction,
    type Notification,
  } from "$lib/stores/notifications.js";

  let { notification }: { notification: Notification } = $props();

  function relativeTime(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return "now";
    if (mins < 60) return `${mins}m`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h`;
    const days = Math.floor(hours / 24);
    return `${days}d`;
  }

  function handleDismiss(e: MouseEvent) {
    e.stopPropagation();
    dismissNotification(notification.id);
  }

  function handleAction(e: MouseEvent, key: string) {
    e.stopPropagation();
    invokeAction(notification.id, key);
  }
</script>

<div class="notif-item">
  <div class="notif-icon">
    {#if notification.app_icon}
      <img src={notification.app_icon} alt="" class="notif-icon-img" />
    {:else}
      <span class="notif-icon-letter">{notification.app_name.charAt(0).toUpperCase()}</span>
    {/if}
  </div>
  <div class="notif-body-col">
    <div class="notif-title-row">
      <span class="notif-summary">{notification.summary}</span>
      <span class="notif-time">{relativeTime(notification.timestamp)}</span>
    </div>
    {#if notification.body}
      <span class="notif-body">{notification.body}</span>
    {/if}
    {#if notification.actions.length > 0}
      <div class="notif-actions">
        {#each notification.actions as action (action.key)}
          <button class="notif-action-btn" onclick={(e) => handleAction(e, action.key)}>
            {action.label}
          </button>
        {/each}
      </div>
    {/if}
  </div>
  <button class="notif-dismiss" onclick={handleDismiss} aria-label="Dismiss">
    <X size={14} strokeWidth={2} />
  </button>
</div>

<style>
  .notif-item {
    position: relative;
    display: flex;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-shell) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    transition: background-color var(--duration-micro, 100ms) ease, border-color var(--duration-micro, 100ms) ease;
  }
  .notif-item:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
  }

  .notif-icon {
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: var(--radius-chip);
    flex-shrink: 0;
    margin-top: 1px;
  }
  .notif-icon-img {
    width: 16px;
    height: 16px;
    object-fit: contain;
  }
  .notif-icon-letter {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--color-fg-shell);
    opacity: 0.7;
  }

  .notif-body-col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    /* Reserve space for the dismiss button (24px + gap) so long
       summary text doesn't slide under it. */
    padding-right: 28px;
  }

  .notif-title-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .notif-summary {
    font-size: 0.875rem;
    font-weight: 500;
    line-height: 1.3;
    color: var(--color-fg-shell);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .notif-time {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .notif-body {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .notif-dismiss {
    position: absolute;
    top: 8px;
    right: 8px;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    padding: 0;
    transition: background-color var(--duration-micro, 100ms) ease, color var(--duration-micro, 100ms) ease;
  }
  .notif-dismiss:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: var(--color-fg-shell);
  }

  .notif-actions {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .notif-action-btn {
    padding: 6px 10px;
    border-radius: var(--radius-chip);
    font-size: 0.75rem;
    font-weight: 500;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: var(--color-fg-shell);
    transition: background-color var(--duration-micro, 100ms) ease, border-color var(--duration-micro, 100ms) ease;
  }
  .notif-action-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 18%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 25%, transparent);
  }
</style>
