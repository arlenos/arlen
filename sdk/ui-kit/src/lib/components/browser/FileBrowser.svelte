<script lang="ts">
  /// The shared file browser: one controller in, the listing with
  /// selection and activation out. Directory activation navigates
  /// internally (every host wants that); everything else calls
  /// `onactivate` and the host decides (the FM opens, the picker
  /// confirms). Hosted unchanged by the FM app and the confined xdg
  /// picker — nothing in here may assume ambient filesystem access
  /// or a particular window chrome.
  import type { Snippet } from "svelte";
  import type { BrowserState } from "./controller";
  import type { FileEntry } from "./types";
  import { joinPath } from "./types";
  import { Selection } from "./selection";
  import FileList from "./FileList.svelte";

  let {
    controller,
    onactivate,
    onselection,
    filter,
    now,
    icon,
  }: {
    /// The headless browser state; swapping it switches tabs.
    controller: BrowserState;
    /// A non-directory entry was activated (double-click or Enter).
    onactivate?: (entry: FileEntry, path: string) => void;
    /// The selection changed; entries are the selected rows.
    onselection?: (entries: FileEntry[]) => void;
    /// Host-side row filter (the picker's globs); directories always
    /// pass on the host side by convention.
    filter?: (entry: FileEntry) => boolean;
    /// Injectable clock for stable screenshots.
    now?: number;
    /// Icon seam for themed and KG-state icons.
    icon?: Snippet<[FileEntry]>;
  } = $props();

  // Each store ref re-derives when the controller prop swaps (a tab
  // switch), so the `$` subscriptions follow the active tab.
  const path = $derived(controller.path);
  const entries = $derived(controller.entries);
  const loading = $derived(controller.loading);
  const error = $derived(controller.error);
  const sortKey = $derived(controller.sortKey);
  const ascending = $derived(controller.ascending);

  const visible = $derived(filter ? $entries.filter(filter) : $entries);

  // Selection is synchronous view state (the documented exception to
  // the stores rule); it rebases whenever the listing identity
  // changes and is mirrored into a plain Set for the rows.
  const selection = new Selection(0);
  let selectedIndices = $state<ReadonlySet<number>>(new Set());
  let cursorIndex = $state<number | null>(null);
  let listedPath = $state("");

  $effect(() => {
    const p = $path;
    const count = visible.length;
    if (p !== listedPath) {
      listedPath = p;
      selection.rebase(count);
    } else if (count !== selection.size()) {
      selection.rebase(count);
    }
    publish();
  });

  function publish() {
    const set = new Set(selection.indices());
    selectedIndices = set;
    cursorIndex = selection.cursor();
    onselection?.([...set].map((i) => visible[i]).filter(Boolean));
  }

  function onrowevent(kind: "click" | "dblclick" | "contextmenu", i: number, e: MouseEvent) {
    if (kind === "click") {
      if (e.shiftKey) selection.rangeTo(i);
      else if (e.ctrlKey || e.metaKey) selection.toggle(i);
      else selection.click(i);
      publish();
      return;
    }
    if (kind === "contextmenu") {
      if (!selection.isSelected(i)) {
        selection.click(i);
        publish();
      }
      return;
    }
    // dblclick
    const entry = visible[i];
    if (!entry) return;
    activate(entry);
  }

  function activate(entry: FileEntry) {
    if (entry.kind === "directory") {
      void controller.navigate(joinPath($path, entry.name));
      return;
    }
    onactivate?.(entry, joinPath($path, entry.name));
  }
</script>

<div class="file-browser">
  {#if $error}
    <div class="fb-state">
      <span class="fb-state-title">Can't open this folder</span>
      <span class="fb-state-hint">{$error}</span>
    </div>
  {:else if !$loading && visible.length === 0}
    <div class="fb-state">
      <span class="fb-state-title">This folder is empty</span>
    </div>
  {:else}
    <FileList
      entries={visible}
      sortKey={$sortKey}
      ascending={$ascending}
      {selectedIndices}
      {cursorIndex}
      {now}
      {icon}
      onsort={(key) => controller.setSort(key)}
      {onrowevent}
    />
  {/if}
</div>

<style>
  .file-browser {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .fb-state {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    text-align: center;
    padding: 2rem;
  }
  .fb-state-title {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .fb-state-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    max-width: 36ch;
  }
</style>
