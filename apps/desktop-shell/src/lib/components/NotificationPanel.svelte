<script lang="ts">
  import { t } from "$lib/i18n/messages";
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
    loadOlder,
    knownApps,
    historyApp,
    loadKnownApps,
    setHistoryApp,
    historyHasMore,
    historyLoading,
    historyFailed,
    historyCapped,
  } from "$lib/stores/notifications.js";
  import { onMount } from "svelte";
  import NotificationItem from "$lib/components/NotificationItem.svelte";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Bell, Trash2, ChevronDown } from "lucide-svelte";

  let expandedGroups = $state<Set<string>>(new Set());

  /// Materialise the Map<string, Notification[]> into an array so
  /// the template destructures entries readably; the keyed each
  /// reconciles by app name either way.
  const groupEntries = $derived(
    Array.from($groupedNotifications.entries()).filter(
      ([appName]) => $historyApp === null || appName === $historyApp,
    ),
  );

  /// The filter's options: every app the daemon knows, plus the ones on screen.
  ///
  /// The union rather than the daemon's list alone, because a notification that
  /// arrived this second is in the panel before any read has asked the daemon
  /// again - and a filter that cannot name what you are looking at is worse than
  /// no filter.
  const filterApps = $derived(
    Array.from(new Set([...$knownApps, ...$groupedNotifications.keys()])).sort(
      (a, b) => a.localeCompare(b),
    ),
  );

  onMount(loadKnownApps);

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
      <span class="notif-section-title">{$t("sh.notif.title")}</span>
      {#if $notifications.length > 0}
        <span class="notif-section-count">{$notifications.length}</span>
      {/if}
    </div>
    {#if $notifications.length > 0}
      <button
        class="notif-clear-btn"
        onclick={() => clearAll()}
        aria-label={$t("sh.notif.clearAllAria")}
      >
        <Trash2 size={14} strokeWidth={1.75} />
        <span>{$t("sh.notif.clear")}</span>
      </button>
    {/if}
  </div>

  {#if $notifications.length === 0}
    <div class="notif-empty">
      <Bell size={28} strokeWidth={1.25} />
      <span>{$t("sh.notif.empty")}</span>
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
              aria-label={$t("sh.notif.showAllAria", { app: appName })}
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

  <!-- The daemon keeps thirty days of these and the panel started from the
       pending ones alone, so everything a person dismissed was stored and
       unreachable. The footer is under BOTH branches on purpose: somebody
       looking at an empty panel is exactly the one who wants back the
       notification they just dismissed. -->
  <div class="notif-history">
    {#if filterApps.length > 1}
      <!-- Only when there is a choice to make. One app is not a filter. -->
      <PopoverSelect
        value={$historyApp ?? ""}
        options={[
          { value: "", label: $t("sh.notif.fromAll") },
          ...filterApps.map((a) => ({ value: a, label: a })),
        ]}
        ariaLabel={$t("sh.notif.fromAria")}
        onchange={(v) => void setHistoryApp(v === "" ? null : v)}
      />
    {/if}
    {#if $historyFailed}
      <p class="notif-history-note" role="alert">{$t("sh.notif.olderFailed")}</p>
    {:else if $historyCapped}
      <p class="notif-history-note">{$t("sh.notif.olderCapped")}</p>
    {:else if $historyHasMore}
      <button class="notif-history-btn" disabled={$historyLoading} onclick={() => void loadOlder()}>
        {$historyLoading ? $t("sh.notif.olderLoading") : $t("sh.notif.older")}
      </button>
    {:else}
      <p class="notif-history-note">{$t("sh.notif.olderNone")}</p>
    {/if}
  </div>
</div>

<style>
  .notif-history {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding-top: 2px;
  }
  .notif-history-btn {
    padding: 4px 10px;
    border-radius: 6px;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    transition: background 0.1s, color 0.1s;
  }
  .notif-history-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
  .notif-history-btn:disabled {
    opacity: 0.6;
  }
  /* 55%, the readable floor this shell settled on for a line that is sometimes
     the only thing in its region. */
  .notif-history-note {
    margin: 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
  }

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
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--color-fg-shell);
    letter-spacing: -0.01em;
  }
  .notif-section-count {
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    border-radius: var(--radius-full, 9999px);
    padding: 0 7px;
    font-size: var(--text-2xs);
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
    font-size: var(--text-xs);
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
    /* 50%, not 35%. Over the panel's own ground (#0a0a0a) a 35% fade measures
       3.05:1 and small text needs 4.5; 50% is the first step that clears it, at
       5.15. It is also the only sentence on the surface when it shows, so a
       quiet-because-there-is-nothing-here fade was making the one line that
       explains the emptiness the hardest thing on the panel to read. */
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    font-size: var(--text-sm);
  }

  .notif-list {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-height: 60vh;
    overflow-y: auto;
    scrollbar-gutter: stable;
    padding-inline-end: 2px;
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
    text-align: start;
    border-radius: var(--radius-chip);
    transition: background-color var(--duration-micro, 100ms) ease;
  }
  .notif-group-header.collapsible:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
  }

  .notif-group-dot {
    width: 6px;
    height: 6px;
    /* Chip radius, not a circle: the house dot family. */
    border-radius: var(--radius-chip, 4px);
    background: var(--color-accent);
    flex-shrink: 0;
  }
  .notif-group-name {
    font-size: var(--text-xs);
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-shell) 80%, transparent);
  }
  .notif-group-count {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    border-radius: var(--radius-full, 9999px);
    padding: 0 6px;
    font-size: var(--text-2xs);
    font-weight: 600;
    line-height: 1.5;
    font-variant-numeric: tabular-nums;
  }
  :global(.notif-chevron) {
    margin-inline-start: auto;
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
