<script lang="ts">
  /// Notifications list inside the NotificationsPopover.
  ///
  /// Grouped by app name. The section header shows "Notifications"
  /// + total count + clear-all button. Each group has its own
  /// header with app name + count pill and (when 3+ items) a
  /// chevron to collapse / expand. Items render as cards via
  /// NotificationItem.

  import {
    notifications,
    groupedNotifications,
    clearAll,
  } from "$lib/stores/notifications.js";
  import NotificationItem from "$lib/components/NotificationItem.svelte";
  import { Bell, Trash2, ChevronDown } from "lucide-svelte";

  let expandedGroups = $state<Set<string>>(new Set());

  /// Materialise the Map<string, Notification[]> into an array so
  /// the template destructures entries readably; the keyed each
  /// reconciles by app name either way.
  const groupEntries = $derived(
    Array.from($groupedNotifications.entries()),
  );

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
  <!-- Section header: title + count + clear-all -->
  <div class="notif-section-header">
    <div class="notif-section-title-row">
      <span class="notif-section-title">Notifications</span>
      {#if $notifications.length > 0}
        <span class="notif-section-count">{$notifications.length}</span>
      {/if}
    </div>
    {#if $notifications.length > 0}
      <button
        class="notif-clear-btn"
        onclick={() => clearAll()}
        aria-label="Clear all notifications"
      >
        <Trash2 size={14} strokeWidth={1.75} />
        <span>Clear</span>
      </button>
    {/if}
  </div>

  {#if $notifications.length === 0}
    <div class="notif-empty">
      <Bell size={28} strokeWidth={1.25} />
      <span>No notifications</span>
    </div>
  {:else}
    <div class="notif-list">
      {#each groupEntries as [appName, items] (appName)}
        <div class="notif-group">
          {#snippet groupHeaderContent()}
            <span class="notif-group-dot"></span>
            <span class="notif-group-name">{appName}</span>
            {#if items.length > 1}
              <span class="notif-group-count">{items.length}</span>
            {/if}
            {#if items.length >= 3}
              <ChevronDown
                size={12}
                strokeWidth={2}
                class="notif-chevron {expandedGroups.has(appName) ? 'expanded' : ''}"
              />
            {/if}
          {/snippet}
          {#if items.length >= 3}
            <button
              class="notif-group-header collapsible"
              aria-expanded={expandedGroups.has(appName)}
              aria-label="Show all {appName} notifications"
              onclick={() => toggleGroup(appName)}
            >
              {@render groupHeaderContent()}
            </button>
          {:else}
            <div class="notif-group-header">
              {@render groupHeaderContent()}
            </div>
          {/if}

          <div class="notif-group-items">
            {#if items.length >= 3 && !expandedGroups.has(appName)}
              <NotificationItem notification={items[0]} />
            {:else}
              {#each items as notif (notif.id)}
                <NotificationItem notification={notif} />
              {/each}
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .notif-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  /* Section header: full-weight title with a count pill and a
     labelled clear-all button. */
  .notif-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 2px;
  }
  .notif-section-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .notif-section-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-fg-shell);
    letter-spacing: -0.01em;
  }
  .notif-section-count {
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    border-radius: var(--radius-full, 9999px);
    padding: 0 7px;
    font-size: 0.6875rem;
    font-weight: 600;
    line-height: 1.5;
    font-variant-numeric: tabular-nums;
  }
  .notif-clear-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--color-fg-shell) 65%, transparent);
    font-size: 0.75rem;
    font-weight: 500;
    transition: background-color var(--duration-micro, 100ms) ease, color var(--duration-micro, 100ms) ease, border-color var(--duration-micro, 100ms) ease;
  }
  .notif-clear-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    color: var(--color-fg-shell);
  }

  .notif-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 32px 0;
    color: color-mix(in srgb, var(--color-fg-shell) 35%, transparent);
    font-size: 0.8125rem;
  }

  .notif-list {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-height: 60vh;
    overflow-y: auto;
    scrollbar-gutter: stable;
    padding-right: 2px;
  }

  .notif-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* Group header: accent dot + app name + count pill. Collapsible
     when 3+ items (chevron rotates). */
  .notif-group-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 6px;
    border: none;
    background: transparent;
    text-align: left;
    border-radius: var(--radius-chip);
    transition: background-color var(--duration-micro, 100ms) ease;
  }
  .notif-group-header.collapsible:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
  }

  .notif-group-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full, 9999px);
    background: var(--color-accent);
    flex-shrink: 0;
  }
  .notif-group-name {
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-shell) 80%, transparent);
  }
  .notif-group-count {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    border-radius: var(--radius-full, 9999px);
    padding: 0 6px;
    font-size: 0.625rem;
    font-weight: 600;
    line-height: 1.5;
    font-variant-numeric: tabular-nums;
  }
  :global(.notif-chevron) {
    margin-left: auto;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    transition: transform var(--duration-fast, 150ms) ease;
  }
  :global(.notif-chevron.expanded) {
    transform: rotate(180deg);
  }

  .notif-group-items {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
