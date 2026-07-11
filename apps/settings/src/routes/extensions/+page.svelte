<script lang="ts">
  /// Extensions panel.
  ///
  /// Lists modules discovered in `/usr/share/arlen/modules/` (system)
  /// and `~/.local/share/arlen/modules/` (user), merged with the
  /// enabled/disabled state from `~/.config/arlen/modules.toml`.
  ///
  /// The shell reads the same `modules.toml`, so a toggle here shows a
  /// "restart required" banner — the change is persisted immediately
  /// but the shell has to be restarted to actually load or unload the
  /// module at runtime.

  import { onMount } from "svelte";
  import { RefreshCw, Puzzle, Info, ExternalLink } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { t } from "$lib/i18n/messages";
  import ModuleCard from "$lib/components/appearance/ModuleCard.svelte";
  import { modules, moduleGroups } from "$lib/stores/modules";

  /// Where installd drops bundled modules and where users can drop
  /// their own. Shown verbatim in the empty state so the user knows
  /// exactly where to put new modules.
  const USER_MODULES_DIR = "~/.local/share/arlen/modules/";
  /// Link to the module-system spec shipped with the repo. When the
  /// Arlen docs site goes live this should flip to the canonical URL.
  const MODULES_DOCS =
    "https://github.com/arlenos/docs/blob/main/architecture/module-system.md";

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

<Page
  title={$t("s.ext.title")}
  description={$t("s.ext.desc")}
>
  <SectionGrid>
  <div class="span-full ext-column">
  <div class="ext-toolbar">
    <IconAction label={$t("s.ext.rescan")} onclick={() => modules.load()}>
      <RefreshCw size={14} strokeWidth={2} />
    </IconAction>
  </div>

  {#if $modules.restartRequired}
    <div class="banner">
      <Info size={12} strokeWidth={2.25} />
      <span>
        {$t("s.ext.restart")}
      </span>
      <Button variant="ghost" size="sm" onclick={() => modules.dismissRestartBanner()}>
        {$t("s.ext.dismiss")}
      </Button>
    </div>
  {/if}

  {#if $modules.loading && $modules.data.length === 0}
    <div class="status">{$t("s.ext.scanning")}</div>
  {:else if $modules.error}
    <div class="error" title={$modules.error}>{$t("s.ext.error")}</div>
  {:else if total === 0}
    <div class="empty">
      <div class="empty-icon">
        <Puzzle size={28} strokeWidth={1.5} />
      </div>
      <h2>{$t("s.ext.noModules")}</h2>
      <p>
        {$t("s.ext.install.pre")} <code>forage install</code>{$t("s.ext.install.mid")}<code>{USER_MODULES_DIR}</code>{$t("s.ext.install.post")}
      </p>
      <a
        class="empty-link"
        href={MODULES_DOCS}
        target="_blank"
        rel="noopener noreferrer"
      >
        {$t("s.ext.learn")}
        <ExternalLink size={12} strokeWidth={2} />
      </a>
    </div>
  {:else}
    <div class="summary">
      <span>{$t("s.ext.summary", { enabled: enabledCount, total })}</span>
    </div>

    <div class="search-wrap">
      <Input placeholder={$t("s.ext.filter")} bind:value={filter} />
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
          {$t("s.ext.noMatchPre")}<strong>{filter}</strong>{$t("s.ext.noMatchPost")}
        </div>
      {/if}
    </div>
  {/if}
  </div>
  </SectionGrid>
</Page>

<style>
  /* Single-column flow inside the grid (cap + centring come from SectionGrid). */
  .ext-column {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .ext-toolbar {
    display: flex;
    justify-content: flex-end;
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

  .summary {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    margin-bottom: 0.5rem;
  }

  .search-wrap {
    margin-bottom: 1rem;
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
