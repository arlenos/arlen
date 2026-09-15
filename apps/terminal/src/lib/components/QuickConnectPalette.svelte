<script lang="ts">
  /// The quick-connect palette (terminal.md §4.12): a fuzzy finder over saved
  /// (KG-grouped by project) + recent hosts, with a free-text `user@host` row for
  /// connect-once, on the kit command primitive like the history palette. The
  /// terminal is the trigger; the Connections panel owns the hosts. No persistent
  /// host tree (deferred as clutter).
  import {
    Command,
    CommandInput,
    CommandList,
    CommandItem,
  } from "@arlen/ui-kit/components/ui/command";
  import { Star } from "lucide-svelte";
  import { trapFocus } from "@arlen/ui-kit/keyboard";
  import { t } from "$lib/i18n/messages";
  import {
    savedHosts,
    recentHosts,
    remotesMocked,
    paletteOpen,
    query,
    closeQuickConnect,
    connectSaved,
    connectAdHoc,
    promoteToSaved,
    type SavedHost,
    type RecentHost,
  } from "$lib/stores/remoteConnections";

  const q = $derived($query.trim().toLowerCase());
  function matches(hay: string): boolean {
    return q.length === 0 || hay.toLowerCase().includes(q);
  }

  const savedMatched = $derived(
    $savedHosts.filter((h) => matches(`${h.user}@${h.host} ${h.label} ${h.project ?? ""}`)),
  );

  interface Group {
    key: string;
    label: string;
    hosts: SavedHost[];
  }
  const savedGroups = $derived.by<Group[]>(() => {
    const by = new Map<string, Group>();
    for (const h of savedMatched) {
      const key = h.project ?? "__other";
      let g = by.get(key);
      if (!g) {
        g = { key, label: h.project ? $t("term.qc.project", { name: h.project }) : $t("term.qc.otherHosts"), hosts: [] };
        by.set(key, g);
      }
      g.hosts.push(h);
    }
    return [...by.values()];
  });

  const recentMatched = $derived($recentHosts.filter((h) => matches(`${h.user}@${h.host}`)));

  // The free-text row shows once the query looks like a host (has a dot or an @),
  // so connect-once never needs a saved entry.
  const freeText = $derived(q.length > 0 && (/[.@]/.test($query.trim())) ? $query.trim() : null);


  // The value bits-ui reports for the highlighted row. Saving a host used to be a
  // button INSIDE the row, which a listbox forbids (an `option` may not contain a
  // control) and which no keyboard could reach anyway - the palette's focus lives
  // in the input and arrow keys move the selection, so Tab never landed on it.
  // The action now sits beside the list, on whatever row is highlighted.
  let highlighted = $state("");
  const highlightedRecent = $derived<RecentHost | null>(
    recentMatched.find((r) => `recent-${r.id}` === highlighted) ?? null,
  );

  function saveHighlighted() {
    if (highlightedRecent) promoteToSaved(highlightedRecent);
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (!$paletteOpen) return;
    if (e.key === "Escape") {
      e.preventDefault();
      closeQuickConnect();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && highlightedRecent) {
      e.preventDefault();
      saveHighlighted();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if $paletteOpen}
  <div
    class="qc-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeQuickConnect();
    }}
  >
    <div
      class="qc-card"
      use:trapFocus
      role="dialog"
      aria-modal="true"
      aria-label={$t("term.qc.aria")}
      tabindex="-1"
    >
      <Command shouldFilter={false} bind:value={highlighted}>
        <CommandInput placeholder={$t("term.qc.placeholder")} autofocus bind:value={$query} />
        {#if $remotesMocked}
          <!-- These read as saved SSH connections; unlabelled a user will try to
               connect to hosts they never configured. -->
          <p class="qc-sample">{$t("term.qc.sample")}</p>
        {/if}
        <CommandList class="qc-list">
          {#if savedGroups.length === 0 && recentMatched.length === 0 && !freeText}
            <div class="qc-empty">{$t("term.qc.empty")}</div>
          {/if}

          {#each savedGroups as g (g.key)}
            <div class="qc-group">{g.label}</div>
            {#each g.hosts as h (h.id)}
              <CommandItem value={`saved-${h.id}`} onSelect={() => connectSaved(h)}>
                <span class="qc-badge"></span>
                <span class="qc-name">{h.label}</span>
                <span class="qc-addr">{h.user}@{h.host}</span>
                {#if h.lastUsed}<span class="qc-meta">{h.lastUsed}</span>{/if}
              </CommandItem>
            {/each}
          {/each}

          {#if recentMatched.length > 0}
            <div class="qc-group">{$t("term.qc.recent")}</div>
            {#each recentMatched as r (r.id)}
              <CommandItem value={`recent-${r.id}`} onSelect={() => connectAdHoc(`${r.user}@${r.host}`)}>
                <span class="qc-badge qc-badge-dim"></span>
                <span class="qc-addr qc-name">{r.user}@{r.host}</span>
                <span class="qc-meta">{r.lastUsed}</span>
              </CommandItem>
            {/each}
          {/if}

          {#if freeText}
            <div class="qc-group">{$t("term.qc.connectOnce")}</div>
            <CommandItem value="freetext" onSelect={() => connectAdHoc(freeText)}>
              <span class="qc-badge qc-badge-dim"></span>
              <span class="qc-name">{$t("term.qc.connectTo")} <span class="qc-addr">{freeText}</span></span>
            </CommandItem>
          {/if}
        </CommandList>
        <div class="qc-foot">
          <span>{$t("term.qc.enterConnects")}</span>
          <span>{$t("term.escCloses")}</span>
          {#if highlightedRecent}
            <button
              class="qc-save"
              aria-label={$t("term.qc.saveAria", { host: highlightedRecent.host })}
              onmousedown={(e) => e.preventDefault()}
              onclick={saveHighlighted}
            >
              <Star size={12} strokeWidth={2} />
              <span>{$t("term.qc.saveHost")}</span>
              <span class="qc-key">{$t("term.qc.saveShortcut")}</span>
            </button>
          {/if}
        </div>
      </Command>
    </div>
  </div>
{/if}

<style>
  .qc-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 20vh;
    background: var(--color-bg-overlay);
  }
  .qc-card {
    width: min(600px, calc(100vw - 48px));
    border: 1px solid color-mix(in srgb, var(--foreground) 15%, transparent);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }
  :global(.qc-list) {
    max-height: 340px;
    padding: 4px;
    scrollbar-width: none;
  }
  /* Directly under the input, above every host row it qualifies. */
  .qc-sample {
    margin: 0;
    padding: 8px 10px 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .qc-group {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 4px;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--color-fg-secondary, #a1a1aa);
  }
  .qc-badge {
    width: 8px;
    height: 8px;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .qc-badge-dim {
    background: color-mix(in srgb, var(--foreground) 22%, transparent);
  }
  .qc-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    white-space: nowrap;
  }
  .qc-addr {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .qc-meta {
    flex-shrink: 0;
    font-size: var(--text-2xs);
    color: var(--color-fg-secondary, #a1a1aa);
  }
  .qc-empty {
    padding: 1.25rem;
    text-align: center;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .qc-foot {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 12px;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    font-size: var(--text-xs);
    color: var(--color-fg-secondary, #a1a1aa);
  }
  /* Pushed to the far end so the two standing hints keep their reading order and
     the one thing that acts sits apart from them. */
  .qc-save {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: auto;
    padding: 2px 8px;
    border: 1px solid color-mix(in srgb, var(--foreground) 15%, transparent);
    border-radius: var(--radius-chip);
    background: transparent;
    font-size: var(--text-xs);
    color: var(--foreground);
  }
  .qc-save:hover {
    color: var(--color-warning);
    border-color: color-mix(in srgb, var(--color-warning) 45%, transparent);
  }
  .qc-key {
    color: var(--color-fg-secondary, #a1a1aa);
  }
</style>
