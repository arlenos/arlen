<script lang="ts">
  /// Single notification item inside a grouped notification list.

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
    <span class="notif-summary">{notification.summary}</span>
    {#if notification.body}
      <span class="notif-body">{notification.body}</span>
    {/if}
    {#if notification.actions.length > 0}
      <div class="notif-actions">
        {#each notification.actions as action}
          <button class="notif-action-btn" onclick={(e) => handleAction(e, action.key)}>
            {action.label}
          </button>
        {/each}
      </div>
    {/if}
  </div>
  <div class="notif-meta">
    <span class="notif-time">{relativeTime(notification.timestamp)}</span>
    <button class="notif-dismiss" onclick={handleDismiss} aria-label="Dismiss">
      <X size={12} strokeWidth={2} />
    </button>
  </div>
</div>

<style>
  .notif-item {
    display: flex;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 6px;
    transition: background-color 100ms ease;
  }
  .notif-item:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }

  .notif-icon {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: 4px;
    flex-shrink: 0;
    margin-top: 1px;
  }
  .notif-icon-img {
    width: 14px;
    height: 14px;
    object-fit: contain;
  }
  .notif-icon-letter {
    font-size: 0.5625rem;
    font-weight: 600;
    color: var(--color-fg-shell);
    opacity: 0.6;
  }

  .notif-body-col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .notif-summary {
    font-size: 0.75rem;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .notif-body {
    font-size: 0.6875rem;
    opacity: 0.5;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .notif-meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    flex-shrink: 0;
  }
  .notif-time {
    font-size: 0.625rem;
    opacity: 0.35;
  }
  .notif-dismiss {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: color-mix(in srgb, var(--color-fg-shell) 40%, transparent);
    cursor: pointer;
    padding: 0;
    opacity: 0;
    transition: opacity 100ms ease, background-color 100ms ease;
  }
  .notif-item:hover .notif-dismiss {
    opacity: 1;
  }
  .notif-dismiss:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: var(--color-fg-shell);
  }

  .notif-actions {
    display: flex;
    gap: 4px;
    margin-top: 4px;
  }
  .notif-action-btn {
    padding: 3px 8px;
    border-radius: 4px;
    font-size: 0.625rem;
    font-weight: 500;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: var(--color-fg-shell);
    cursor: pointer;
    transition: background-color 100ms ease;
  }
  .notif-action-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
  }
</style>
