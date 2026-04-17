<script lang="ts">
  /// Notification section inside QuickSettingsPanel.
  ///
  /// Groups by app name. Each group has a header with app name and count.
  /// Groups with 3+ items collapse to show only the latest.

  import {
    notifications,
    groupedNotifications,
    clearAll,
  } from "$lib/stores/notifications.js";
  import NotificationItem from "$lib/components/NotificationItem.svelte";
  import { Bell, Trash2, ChevronDown } from "lucide-svelte";

  let expandedGroups = $state<Set<string>>(new Set());

  function toggleGroup(key: string) {
    expandedGroups = new Set(expandedGroups);
    if (expandedGroups.has(key)) {
      expandedGroups.delete(key);
    } else {
      expandedGroups.add(key);
    }
  }
</script>

<div class="notif-section">
  <!-- Section header -->
  <div class="notif-section-header">
    <span class="notif-section-title">Notifications</span>
    {#if $notifications.length > 0}
      <button class="notif-clear-btn" onclick={() => clearAll()} title="Clear all">
        <Trash2 size={12} strokeWidth={2} />
      </button>
    {/if}
  </div>

  {#if $notifications.length === 0}
    <div class="notif-empty">
      <Bell size={24} strokeWidth={1} />
      <span>No notifications</span>
    </div>
  {:else}
    <div class="notif-list">
      {#each [...$groupedNotifications.entries()] as [appName, items] (appName)}
        <!-- Group header (always shown) -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="notif-group-header"
          class:collapsible={items.length >= 3}
          onclick={() => { if (items.length >= 3) toggleGroup(appName); }}
        >
          <span class="notif-group-name">{appName}</span>
          {#if items.length > 1}
            <span class="notif-group-count">{items.length}</span>
          {/if}
          {#if items.length >= 3}
            <ChevronDown
              size={10}
              strokeWidth={2}
              class="notif-chevron {expandedGroups.has(appName) ? 'expanded' : ''}"
            />
          {/if}
        </div>

        <!-- Items -->
        {#if items.length >= 3 && !expandedGroups.has(appName)}
          <NotificationItem notification={items[0]} />
        {:else}
          {#each items as notif (notif.id)}
            <NotificationItem notification={notif} />
          {/each}
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .notif-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .notif-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .notif-section-title {
    font-size: 0.6875rem;
    font-weight: 600;
    opacity: 0.5;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .notif-clear-btn {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: color-mix(in srgb, var(--color-fg-shell) 40%, transparent);
    cursor: pointer;
    padding: 0;
    transition: all 100ms ease;
  }
  .notif-clear-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }

  .notif-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 16px 0;
    opacity: 0.25;
    font-size: 0.6875rem;
  }

  .notif-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: 280px;
    overflow-y: auto;
    scrollbar-gutter: stable;
    padding-right: 2px;
  }

  .notif-group-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    margin-top: 2px;
  }
  .notif-group-header.collapsible {
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background-color 100ms ease;
  }
  .notif-group-header.collapsible:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
  }
  .notif-group-header:first-child {
    margin-top: 0;
  }

  .notif-group-name {
    font-size: 0.625rem;
    font-weight: 600;
    opacity: 0.5;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .notif-group-count {
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    padding: 0 5px;
    border-radius: var(--radius-md);
    font-size: 0.5625rem;
    line-height: 15px;
    opacity: 0.5;
  }
  :global(.notif-chevron) {
    margin-left: auto;
    opacity: 0.4;
    transition: transform 150ms ease;
  }
  :global(.notif-chevron.expanded) {
    transform: rotate(180deg);
  }
</style>
