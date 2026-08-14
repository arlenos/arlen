<script lang="ts">
  /// The search row under the headerbar. Two modes when the assistant is on: a
  /// literal name search (the query field + the collapsed Filter) and "Ask
  /// Arlen", a natural-language question scoped to this folder that drafts a
  /// facet filter. The mode toggle only shows when the assistant is enabled, and
  /// when it is not the bar says which of the two reasons applies rather than
  /// simply dropping the affordance - a person who has heard of Ask Arlen and
  /// finds nothing here is owed the difference between "off" and "not answering".
  import { tick } from "svelte";
  import { X } from "lucide-svelte";
  import { Search, Sparkles } from "@lucide/svelte";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import FmFilterMenu from "$lib/components/FmFilterMenu.svelte";
  import { t } from "$lib/i18n/messages";
  import {
    closeSearch,
    queueSearch,
    searchOpen,
    searchQuery,
  } from "$lib/stores/search";
  import {
    askMode,
    aiEnabled,
    askLoading,
    askCapability,
    askCapabilityLoaded,
    askCapabilityMessage,
  } from "$lib/stores/ask";

  let {
    path,
    onsave,
    onask,
  }: {
    /// The location the search runs under.
    path: string;
    /// Save the current query as a sidebar search.
    onsave?: (query: string) => void;
    /// Send a natural-language ask scoped to this folder (the page drafts the
    /// facets, navigates, and shows the banner).
    onask?: (query: string) => void;
  } = $props();

  let inputRef = $state<HTMLInputElement | null>(null);
  let askValue = $state("");
  const asking = $derived($askMode === "ask" && $aiEnabled);

  $effect(() => {
    if ($searchOpen) {
      tick().then(() => inputRef?.focus());
    }
  });
  // Re-focus the field when the mode flips so the next keystroke lands there.
  $effect(() => {
    void $askMode;
    if ($searchOpen) tick().then(() => inputRef?.focus());
  });

  function close() {
    askMode.set("search");
    askValue = "";
    closeSearch();
  }

  function submitAsk() {
    const q = askValue.trim();
    if (q.length > 0) onask?.(q);
  }
</script>

{#if $searchOpen}
  <div class="search-wrap">
  <div class="search-bar">
    {#if $aiEnabled}
      <SegmentedControl
        options={[
          { value: "search", label: $t("f.search.search"), icon: Search },
          { value: "ask", label: $t("f.search.askArlen"), icon: Sparkles },
        ]}
        bind:value={$askMode}
        ariaLabel={$t("f.search.mode")}
      />
    {/if}

    {#if asking}
      <Input
        id="files-ask-input"
        bind:ref={inputRef}
        bind:value={askValue}
        class="h-7 text-xs"
        placeholder={$t("f.search.askPlaceholder")}
        aria-label={$t("f.search.askArlen")}
        disabled={$askLoading}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            submitAsk();
          } else if (e.key === "Escape") {
            e.preventDefault();
            close();
          }
        }}
      />
    {:else}
      <SearchField
        id="files-search-input"
        bind:ref={inputRef}
        bind:value={$searchQuery}
        placeholder={$t("f.search.searchPlaceholder")}
        aria-label={$t("f.search.search")}
        oninput={() => queueSearch(path)}
        onkeydown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            close();
          }
        }}
      />
      <FmFilterMenu {path} {onsave} />
    {/if}

    <button class="sb-close" aria-label={$t("f.search.close")} onclick={() => close()}>
      <X size={14} strokeWidth={2} />
    </button>
  </div>

  <!-- Only the two failing states speak. A capability that has not been read yet
       says nothing either, because "off" would be a claim we have not checked. -->
  {#if $askCapabilityLoaded && !$aiEnabled}
    <p class="sb-cap">{$t(askCapabilityMessage($askCapability))}</p>
  {/if}
  </div>
{/if}

<style>
  .search-wrap {
    display: flex;
    flex-direction: column;
  }
  .search-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 8px;
  }
  /* Sits under the row rather than inside it: the row is a control strip, and a
     sentence squeezed between controls is read as a label for one of them. */
  .sb-cap {
    margin: 0;
    padding: 0 8px 8px;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .search-bar :global(input) {
    flex: 1;
  }
  /* Flat-house focus: the kit Input's accent glow ring is replaced by a
     quiet border shift, the register the rest of the chrome uses. */
  .search-bar :global(input:focus-visible) {
    box-shadow: none;
    border-color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }

  .sb-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .sb-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
</style>
