<script module lang="ts">
  /// The inline-eval result shape lives with the store that
  /// produces it.
  import type { WaypointerResult as InlineEvalResult } from "$lib/stores/waypointerActions.js";
  export type { InlineEvalResult };

  /// Prefix-triggered modes: '>' shell, '#' manual, '?' web search,
  /// plus the detected url / kill / unicode / projects modes.
  export type SpecialMode =
    | "shell"
    | "man"
    | "url"
    | "search"
    | "kill"
    | "unicode"
    | "projects"
    | null;
</script>

<script lang="ts">
  /// The inline result card above the scrollable list: calculator /
  /// unit / date evaluations and the prefix-mode echoes (shell, man,
  /// web search, url).
  ///
  /// LOAD-BEARING: the host's 150ms poll effect writes this card's
  /// visibility and text DIRECTLY into the DOM by id
  /// (`wp-inline-wrap` / `wp-inline-result` / `wp-inline-hint`) —
  /// the project-documented workaround for `$state` mutated from
  /// IPC callbacks not re-rendering reliably. Only the ICON is
  /// Svelte-rendered (from the stores below). The ids and the
  /// always-in-DOM mounting must not change.
  import type { Writable } from "svelte/store";
  import {
    TerminalSquare,
    BookOpen,
    Globe,
    Search,
    Skull,
    Clock,
    ArrowRightLeft,
    Calculator,
  } from "lucide-svelte";

  let {
    specialMode,
    inlineResult,
    onActivate,
  }: {
    specialMode: Writable<SpecialMode>;
    inlineResult: Writable<InlineEvalResult | null>;
    /// Click dispatch — the host owns the per-mode actions.
    onActivate: () => void;
  } = $props();
</script>

<!-- Inline result: above the scrollable list, always in DOM -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div id="wp-inline-wrap" style="display: none; padding: 6px 6px 2px;">
  <div class="wp-inline-card" onclick={onActivate}>
    <span id="wp-inline-icon" class="wp-inline-icon">
      {#if $specialMode === "shell"}
        <TerminalSquare size={18} strokeWidth={1.5} />
      {:else if $specialMode === "man"}
        <BookOpen size={18} strokeWidth={1.5} />
      {:else if $specialMode === "url"}
        <Globe size={18} strokeWidth={1.5} />
      {:else if $specialMode === "search"}
        <Search size={18} strokeWidth={1.5} />
      {:else if $specialMode === "kill"}
        <Skull size={18} strokeWidth={1.5} />
      {:else if $inlineResult?.result_type === "datetime"}
        <Clock size={18} strokeWidth={1.5} />
      {:else if $inlineResult?.result_type === "unit"}
        <ArrowRightLeft size={18} strokeWidth={1.5} />
      {:else}
        <Calculator size={18} strokeWidth={1.5} />
      {/if}
    </span>
    <span id="wp-inline-result" class="wp-inline-result"></span>
    <span id="wp-inline-hint" class="wp-inline-hint">Copy</span>
  </div>
</div>

<style>
  .wp-inline-card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    color: var(--color-fg-shell);
  }

  .wp-inline-result {
    font-size: 1.125rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .wp-inline-hint {
    margin-left: auto;
    font-size: 0.6875rem;
    opacity: 0.35;
  }
</style>
