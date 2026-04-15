<script lang="ts">
  /// Extensions panel.
  ///
  /// Lists modules discovered in `/usr/share/lunaris/modules/` (system)
  /// and `~/.local/share/lunaris/modules/` (user), merged with the
  /// enabled/disabled state from `~/.config/lunaris/modules.toml`.
  ///
  /// The shell reads the same `modules.toml`, so a toggle here shows a
  /// "restart required" banner — the change is persisted immediately
  /// but the shell has to be restarted to actually load or unload the
  /// module at runtime.

  import { onMount } from "svelte";
  import { RefreshCw, Puzzle, Info } from "lucide-svelte";
  import Group from "$lib/components/appearance/Group.svelte";
  import ModuleCard from "$lib/components/appearance/ModuleCard.svelte";
  import { modules, moduleGroups } from "$lib/stores/modules";

  let filter = $state("");

  onMount(() => {
    modules.load();
  });

  // Filter each group in-place based on the search query.
  const filteredGroups = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return $moduleGroups;
    return $moduleGroups
      .map((g) => ({
        label: g.label,
        items: g.items.filter(
          (m) =>
            m.name.toLowerCase().includes(q) ||
            m.id.toLowerCase().includes(q) ||
            m.description.toLowerCase().includes(q),
        ),
      }))
      .filter((g) => g.items.length > 0);
  });

  const total = $derived($modules.data.length);
  const enabledCount = $derived(
    $modules.data.filter((m) => m.enabled).length,
  );
</script>

<div class="page">
  <header class="head">
    <div class="head-text">
      <h1>Extensions</h1>
      <p class="lede">
        Modules that extend the Lunaris shell. Install third-party modules
        by dropping them into
        <code>~/.local/share/lunaris/modules/</code>.
      </p>
    </div>
    <button
      type="button"
      class="refresh-btn"
      onclick={() => modules.load()}
      title="Re-scan module directories"
    >
      <RefreshCw size={12} strokeWidth={2.25} />
    </button>
  </header>

  {#if $modules.restartRequired}
    <div class="banner">
      <Info size={12} strokeWidth={2.25} />
      <span>
        Changes will take effect after the Lunaris shell restarts.
      </span>
      <button
        type="button"
        class="banner-dismiss"
        onclick={() => modules.dismissRestartBanner()}
      >
        Dismiss
      </button>
    </div>
  {/if}

  {#if $modules.loading && $modules.data.length === 0}
    <div class="status">Scanning module directories…</div>
  {:else if $modules.error}
    <div class="error">Failed to load modules: {$modules.error}</div>
  {:else if total === 0}
    <div class="empty">
      <div class="empty-icon">
        <Puzzle size={28} strokeWidth={1.5} />
      </div>
      <h2>No modules installed</h2>
      <p>
        Modules live in <code>~/.local/share/lunaris/modules/&lt;id&gt;/</code>
        or are shipped with the system in
        <code>/usr/share/lunaris/modules/</code>. Each directory must
        contain a <code>manifest.toml</code>.
      </p>
    </div>
  {:else}
    <div class="summary">
      <span>{enabledCount}/{total} modules enabled</span>
    </div>

    <div class="search-wrap">
      <input
        type="text"
        class="search"
        placeholder="Filter modules..."
        value={filter}
        oninput={(e) => (filter = (e.currentTarget as HTMLInputElement).value)}
      />
    </div>

    <div class="groups">
      {#each filteredGroups as group (group.label)}
        <Group label={group.label}>
          <div class="group-inner">
            {#each group.items as m (m.id)}
              <ModuleCard
                module={m}
                onToggle={(enabled) => modules.setEnabled(m.id, enabled)}
                onUninstall={() => modules.uninstall(m.id)}
              />
            {/each}
          </div>
        </Group>
      {/each}

      {#if filter && filteredGroups.length === 0}
        <div class="empty small">
          No modules match "<strong>{filter}</strong>".
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .page {
    width: 100%;
    max-width: 44rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.25rem;
  }
  .head-text {
    flex: 1;
  }
  h1 {
    margin: 0 0 0.25rem;
    font-size: 1.125rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--foreground);
  }
  .lede {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .lede code {
    font-family: var(--font-mono);
    font-size: 0.6875rem;
    padding: 0.05rem 0.3rem;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .refresh-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    cursor: pointer;
    flex-shrink: 0;
    transition: background-color 120ms ease;
  }
  .refresh-btn:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0.6rem 0.75rem;
    margin-bottom: 1rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--color-accent);
    font-size: 0.75rem;
  }
  .banner span {
    flex: 1;
    color: var(--foreground);
  }
  .banner-dismiss {
    background: transparent;
    border: none;
    color: var(--color-accent);
    font: inherit;
    font-size: 0.6875rem;
    text-decoration: underline;
    cursor: pointer;
    padding: 0;
  }

  .summary {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    margin-bottom: 0.5rem;
  }

  .search-wrap {
    margin-bottom: 1rem;
  }
  .search {
    width: 100%;
    height: 30px;
    padding: 0 0.7rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    font: inherit;
    font-size: 0.75rem;
    outline: none;
  }
  .search:focus-visible {
    border-color: color-mix(in srgb, var(--color-accent) 40%, transparent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 18%, transparent);
  }

  .groups {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }
  .group-inner {
    padding: 0.625rem;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .status {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .error {
    padding: 0.75rem 1rem;
    border-radius: var(--radius-md);
    border: 1px solid color-mix(in srgb, var(--color-error) 40%, transparent);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    color: var(--color-error);
    font-size: 0.8125rem;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 0.75rem;
    padding: 3rem 1rem;
    border-radius: var(--radius-lg);
    border: 1px dashed color-mix(in srgb, var(--foreground) 15%, transparent);
    background: color-mix(in srgb, var(--foreground) 2%, transparent);
  }
  .empty.small {
    padding: 1.25rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .empty-icon {
    width: 56px;
    height: 56px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    color: var(--color-accent);
  }
  .empty h2 {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .empty p {
    margin: 0;
    max-width: 32rem;
    font-size: 0.75rem;
    line-height: 1.55;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .empty code {
    font-family: var(--font-mono);
    font-size: 0.6875rem;
    padding: 0.05rem 0.3rem;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .empty strong {
    color: var(--foreground);
  }
</style>
