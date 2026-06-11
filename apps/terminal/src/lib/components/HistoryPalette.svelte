<script lang="ts">
  /// The Ctrl+R history palette (terminal.md §4.10: a fuzzy finder
  /// over the command record with failure, origin and project
  /// filters), on the kit command primitive like the shell's
  /// Waypointer. Picking a row puts the command into the composer;
  /// nothing executes from here.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import {
    Command,
    CommandInput,
    CommandList,
    CommandItem,
  } from "@arlen/ui-kit/components/ui/command";
  import { Toggle } from "@arlen/ui-kit/components/ui/toggle";
  import { terminalProjects, type Block, type Project } from "$lib/contract";
  import { shortPath } from "$lib/paths";
  import { prefillComposer } from "$lib/stores/composer";
  import {
    historyPaletteOpen,
    historyQuery,
    historyOnlyFailures,
    historyAgentOnly,
    historyProjectId,
    historyResults,
    historyLoaded,
    closeHistoryPalette,
    runHistorySearch,
    queueHistorySearch,
  } from "$lib/stores/history";

  /// Project scopes come from the graph-backed contract command; with
  /// none recorded yet the chip row simply ends after Agent.
  const projects = writable<Project[]>([]);

  onMount(async () => {
    try {
      projects.set(await terminalProjects());
    } catch {
      // Unreachable backend: the palette just has no project chips.
    }
  });

  // A fresh search every time the palette opens, so it never shows a
  // stale result set.
  $effect(() => {
    if ($historyPaletteOpen) runHistorySearch();
  });

  function toggleProject(id: string) {
    historyProjectId.update((cur) => (cur === id ? null : id));
    queueHistorySearch();
  }

  function pick(b: Block) {
    prefillComposer(b.command);
    closeHistoryPalette();
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (!$historyPaletteOpen) return;
    if (e.key === "Escape") {
      e.preventDefault();
      closeHistoryPalette();
    }
  }

  const historyFiltered = $derived(
    $historyQuery.trim().length > 0 ||
      $historyOnlyFailures ||
      $historyAgentOnly ||
      $historyProjectId !== null,
  );
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if $historyPaletteOpen}
  <div
    class="hp-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeHistoryPalette();
    }}
  >
    <div
      class="hp-card"
      role="dialog"
      aria-modal="true"
      aria-label="Command history"
      tabindex="-1"
    >
      <Command shouldFilter={false}>
        <CommandInput
          placeholder="Search history"
          autofocus
          bind:value={$historyQuery}
          oninput={() => queueHistorySearch()}
        />
        <div class="hp-chips">
          <Toggle
            id="terminal-history-failures"
            bind:pressed={$historyOnlyFailures}
            class="hp-chip"
            aria-label="Only failed commands"
            onPressedChange={() => queueHistorySearch()}
          >
            Failures
          </Toggle>
          <Toggle
            id="terminal-history-agent"
            bind:pressed={$historyAgentOnly}
            class="hp-chip"
            aria-label="Only commands the agent ran"
            onPressedChange={() => queueHistorySearch()}
          >
            Agent
          </Toggle>
          {#if $projects.length > 0}
            <!-- Attribute filters left, project scopes right of the
                 word: "Failures Agent in Arlen" reads as a sentence. -->
            <span class="hp-in" aria-hidden="true">in</span>
          {/if}
          {#each $projects as p (p.id)}
            <Toggle
              id={`terminal-history-project-${p.id}`}
              pressed={$historyProjectId === p.id}
              class="hp-chip"
              aria-label={`Only commands in ${p.name}`}
              onPressedChange={() => toggleProject(p.id)}
            >
              {p.name}
            </Toggle>
          {/each}
        </div>
        <CommandList class="hp-list">
          {#if $historyLoaded && $historyResults.length === 0}
            <div class="hp-empty">
              {historyFiltered
                ? "No matching commands."
                : "Commands you run land here."}
            </div>
          {/if}
          {#each $historyResults as b (b.id)}
            <CommandItem value={b.id} onSelect={() => pick(b)}>
              <span class="hp-cmd">{b.command}</span>
              <span class="hp-meta">
                {shortPath(b.cwd)}
                {#if b.exit_code !== null && b.exit_code !== 0}
                  <span class="hp-exit">exit {b.exit_code}</span>
                {/if}
              </span>
            </CommandItem>
          {/each}
        </CommandList>
        <div class="hp-foot">
          <span>Enter puts the command into the composer</span>
          <span>Esc closes</span>
        </div>
      </Command>
    </div>
  </div>
{/if}

<style>
  .hp-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 20vh;
    background: var(--color-bg-overlay);
  }

  .hp-card {
    width: min(600px, calc(100vw - 48px));
    border: 1px solid color-mix(in srgb, var(--foreground) 15%, transparent);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }

  .hp-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .hp-chips :global(.hp-chip) {
    height: var(--height-control-compact, 24px);
    padding: 0 7px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-chip);
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .hp-in {
    align-self: center;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .hp-chips :global(.hp-chip[data-state="on"]) {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent, var(--primary)) 35%, transparent);
    color: var(--color-accent, var(--primary));
  }

  :global(.hp-list) {
    max-height: 320px;
    padding: 4px;
    scrollbar-width: none;
  }

  .hp-cmd {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hp-meta {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .hp-exit {
    color: var(--color-error);
    font-variant-numeric: tabular-nums;
  }

  .hp-empty {
    padding: 1.25rem;
    text-align: center;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .hp-foot {
    display: flex;
    gap: 14px;
    padding: 6px 12px;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
</style>
