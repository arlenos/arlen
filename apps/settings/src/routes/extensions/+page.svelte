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
  import { RefreshCw, Puzzle, Info, ExternalLink } from "lucide-svelte";
  import Group from "$lib/components/appearance/Group.svelte";
  import ModuleCard from "$lib/components/appearance/ModuleCard.svelte";
  import { modules, moduleGroups } from "$lib/stores/modules";

  /// Where installd drops bundled modules and where users can drop
  /// their own. Shown verbatim in the empty state so the user knows
  /// exactly where to put new modules.
  const USER_MODULES_DIR = "~/.local/share/lunaris/modules/";
  const SYSTEM_MODULES_DIR = "/usr/share/lunaris/modules/";
  /// Link to the module-system spec shipped with the repo. When the
  /// Lunaris docs site goes live this should flip to the canonical URL.
  const MODULES_DOCS =
    "https://github.com/lunaris-sys/docs/blob/main/architecture/module-system.md";

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
        Modules live in <code>{USER_MODULES_DIR}&lt;id&gt;/</code> for the
        current user, or in <code>{SYSTEM_MODULES_DIR}</code> system-wide.
        Each directory needs a <code>manifest.toml</code>. Install them
        with <code>forage install</code> or drop them in manually; the
        Settings app rescans when you press refresh.
      </p>
      <a
        class="empty-link"
        href={MODULES_DOCS}
        target="_blank"
        rel="noopener noreferrer"
      >
        Learn about modules
        <ExternalLink size={12} strokeWidth={2} />
      </a>
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
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .refresh-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-card);
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
    border-radius: var(--radius-card);
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
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .empty strong {
    color: var(--foreground);
  }
  .empty-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 0.75rem;
    color: var(--color-accent);
    text-decoration: none;
    padding: 0.3rem 0.6rem;
    border-radius: var(--radius-chip);
    transition: background-color 120ms ease;
  }
  .empty-link:hover {
    background: color-mix(in srgb, var(--color-accent) 10%, transparent);
  }
</style>
