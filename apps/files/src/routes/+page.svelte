<script lang="ts">
  /// The browser surface: the shared FileBrowser on the active tab's
  /// controller, the status line — plus the FM-only operations layer:
  /// context menu, clipboard, rename, trash and the permanent-delete
  /// confirmation. The chrome (nav, breadcrumb, tabs, toggles) lives
  /// in the layout's headerbar.
  import { onMount, tick } from "svelte";
  import { get } from "svelte/store";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    FileBrowser,
    joinPath,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { openPath } from "$lib/adapter";
  import { activeController, newTab, tabs } from "$lib/stores/tabs";
  import { focusedController, focusedPane, paneB, splitView } from "$lib/stores/panes";
  import { addBookmark, homePath, loadPlaces } from "$lib/stores/places";
  import { infoOpen } from "$lib/stores/ui";
  import { clipboard, paste, runOp } from "$lib/stores/ops";
  import FmStatusBar from "$lib/components/FmStatusBar.svelte";
  import OpsOverlays from "$lib/components/OpsOverlays.svelte";
  import FmSearchBar from "$lib/components/FmSearchBar.svelte";
  import FmSearchResults from "$lib/components/FmSearchResults.svelte";
  import FmInfoPanel from "$lib/components/FmInfoPanel.svelte";
  import { savedSearches } from "$lib/stores/places";
  import { searchOpen, searchResults } from "$lib/stores/search";

  let renamingName = $state<string | null>(null);
  let confirmDelete = $state(false);

  // What the info panel inspects: the single selected entry, or the
  // folder itself when nothing is selected.
  const infoTarget = $derived.by(() => {
    if (selected.length === 1) {
      return {
        path: joinPath(currentPath(), selected[0].name),
        entry: selected[0],
      };
    }
    return { path: currentPath(), entry: null };
  });

  /// Save the current query into the Searches sidebar group (session
  /// only; persisting needs a contract command, flagged).
  function saveSearch(query: string) {
    savedSearches.update((list) => [
      ...list,
      { id: `local-${list.length + 1}-${query}`, name: query, query },
    ]);
  }

  // Dual pane state lives in the panes store (the headerbar's view
  // controls drive it); the page keeps only the per-pane selections.
  let selectedA = $state<FileEntry[]>([]);
  let selectedB = $state<FileEntry[]>([]);

  const selected = $derived(
    $splitView && $focusedPane === "b" ? selectedB : selectedA,
  );
  $effect(() => {
    if (!$splitView) selectedB = [];
  });

  // The visible listing of the focused pane, mirrored for the status
  // line, plus whether its listing failed (the bar then stays quiet).
  let entries = $state<FileEntry[]>([]);
  let listingError = $state(false);
  $effect(() => {
    const c = $focusedController;
    if (!c) return;
    return c.entries.subscribe((list) => (entries = list));
  });
  $effect(() => {
    const c = $focusedController;
    if (!c) return;
    return c.error.subscribe((e) => (listingError = e !== null));
  });
  // A tab switch shows the new tab's selection state, which starts
  // empty — the browser republishes on interaction.
  $effect(() => {
    void $activeController;
    selectedA = [];
  });

  const currentPath = (): string => {
    const c = get(focusedController);
    return c ? get(c.path) : "/";
  };
  const selectedPaths = (): string[] => {
    const base = currentPath();
    return selected.map((e) => joinPath(base, e.name));
  };

  async function newFolder() {
    const ok = await runOp("new_folder", ["New folder"], currentPath());
    if (ok) {
      await tick();
      renamingName = "New folder";
    }
  }

  function copySelection(kind: "copy" | "move") {
    if (selected.length === 0) return;
    clipboard.set({ kind, paths: selectedPaths() });
  }

  async function trashSelection() {
    if (selected.length === 0) return;
    await runOp("trash", selectedPaths());
  }

  function onOpsKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    if (target?.closest("input, textarea")) return;
    if (e.key === "Tab" && $splitView) {
      e.preventDefault();
      focusedPane.update((p) => (p === "a" ? "b" : "a"));
      const panes = document.querySelectorAll<HTMLElement>(".file-browser");
      panes[get(focusedPane) === "a" ? 0 : 1]?.focus();
      return;
    }
    if (e.key === "Delete" && !e.shiftKey) {
      e.preventDefault();
      void trashSelection();
    } else if (e.key === "Delete" && e.shiftKey) {
      if (selected.length > 0) {
        e.preventDefault();
        confirmDelete = true;
      }
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
      copySelection("copy");
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "x") {
      copySelection("move");
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v") {
      e.preventDefault();
      void paste(currentPath());
    }
  }

  const deleteMessage = $derived(
    selected.length === 1
      ? `Delete ${selected[0]?.name} forever? This cannot be undone.`
      : `Delete ${selected.length} items forever? This cannot be undone.`,
  );

  onMount(async () => {
    await loadPlaces();
    if (get(tabs).length === 0) newTab(get(homePath));
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="fm" onkeydown={onOpsKeydown}>
  {#if $activeController && $focusedController}
    <FmSearchBar path={currentPath()} onsave={saveSearch} />
    <ContextMenu.Root>
      <ContextMenu.Trigger class="fm-browse">
        {#if $searchOpen && $searchResults !== null}
          <FmSearchResults
            basePath={currentPath()}
            onjump={(dir) => $focusedController?.navigate(dir)}
          />
        {:else}
        <div class="fm-panes" class:split={$splitView}>
          <div
            class="fm-pane"
            class:pane-focused={$splitView && $focusedPane === "a"}
            onfocusin={() => focusedPane.set("a")}
          >
            <FileBrowser
              controller={$activeController}
              bind:renamingName
              onactivate={(entry, path) => {
                if (entry.kind !== "directory") void openPath(path);
              }}
              onselection={(list) => (selectedA = list)}
              onrenamecommit={(entry, newName) =>
                runOp("rename", [joinPath(currentPath(), entry.name)], newName)}
            />
          </div>
          {#if $splitView && $paneB}
            <div
              class="fm-pane"
              class:pane-focused={$focusedPane === "b"}
              onfocusin={() => focusedPane.set("b")}
            >
              <FileBrowser
                controller={$paneB}
                onactivate={(entry, path) => {
                  if (entry.kind !== "directory") void openPath(path);
                }}
                onselection={(list) => (selectedB = list)}
                onrenamecommit={(entry, newName) =>
                  runOp("rename", [joinPath(currentPath(), entry.name)], newName)}
              />
            </div>
          {/if}
          {#if $infoOpen}
            <FmInfoPanel
              path={infoTarget.path}
              entry={infoTarget.entry}
              onclose={() => (infoOpen.set(false))}
            />
          {/if}
        </div>
        {/if}
      </ContextMenu.Trigger>
      <ContextMenu.Content class="w-52">
        {#if selected.length > 0}
          <ContextMenu.Item
            onclick={() => {
              const e = selected[0];
              if (!e) return;
              const p = joinPath(currentPath(), e.name);
              if (e.kind === "directory") void $activeController?.navigate(p);
              else void openPath(p);
            }}
          >
            Open
          </ContextMenu.Item>
          {#if selected.length === 1 && selected[0]?.kind === "directory"}
            <ContextMenu.Item onclick={() => newTab(joinPath(currentPath(), selected[0].name))}>
              Open in new tab
            </ContextMenu.Item>
            <ContextMenu.Item
              onclick={() => addBookmark(joinPath(currentPath(), selected[0].name))}
            >
              Pin to sidebar
            </ContextMenu.Item>
          {/if}
          <ContextMenu.Separator />
          <ContextMenu.Item onclick={() => copySelection("copy")}>
            Copy
            <ContextMenu.Shortcut>Ctrl+C</ContextMenu.Shortcut>
          </ContextMenu.Item>
          <ContextMenu.Item onclick={() => copySelection("move")}>
            Cut
            <ContextMenu.Shortcut>Ctrl+X</ContextMenu.Shortcut>
          </ContextMenu.Item>
        {/if}
        <ContextMenu.Item
          disabled={$clipboard === null}
          onclick={() => paste(currentPath())}
        >
          Paste
          <ContextMenu.Shortcut>Ctrl+V</ContextMenu.Shortcut>
        </ContextMenu.Item>
        {#if selected.length > 0}
          <ContextMenu.Separator />
          <ContextMenu.Item onclick={() => runOp("duplicate", selectedPaths())}>
            Duplicate
          </ContextMenu.Item>
          {#if selected.length === 1}
            <ContextMenu.Item onclick={() => (renamingName = selected[0]?.name ?? null)}>
              Rename
              <ContextMenu.Shortcut>F2</ContextMenu.Shortcut>
            </ContextMenu.Item>
          {/if}
        {/if}
        {#if selected.length === 0}
          <ContextMenu.Separator />
          <ContextMenu.Item onclick={newFolder}>New folder</ContextMenu.Item>
        {/if}
        {#if selected.length > 0}
          <ContextMenu.Separator />
          <ContextMenu.Item onclick={trashSelection}>
            Move to trash
            <ContextMenu.Shortcut>Del</ContextMenu.Shortcut>
          </ContextMenu.Item>
          <ContextMenu.Item variant="destructive" onclick={() => (confirmDelete = true)}>
            Delete forever
          </ContextMenu.Item>
        {/if}
      </ContextMenu.Content>
    </ContextMenu.Root>
    <FmStatusBar
      {entries}
      {selected}
      errored={listingError}
      resultsCount={$searchOpen && $searchResults !== null
        ? $searchResults.length
        : null}
    />
  {/if}
</div>

<OpsOverlays />

<ConfirmDialog
  open={confirmDelete}
  title="Delete forever"
  message={deleteMessage}
  confirmLabel="Delete forever"
  variant="destructive"
  onConfirm={async () => {
    await runOp("delete", selectedPaths());
    confirmDelete = false;
  }}
  onCancel={() => (confirmDelete = false)}
/>

<style>
  .fm {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .fm :global(.fm-browse) {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .fm-panes {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .fm-pane {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .fm-panes.split .fm-pane + .fm-pane {
    border-left: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  /* The focused pane carries a quiet top rule so the toolbar's
     subject is visible; only meaningful with two panes. */
  .fm-panes.split .fm-pane.pane-focused {
    box-shadow: inset 0 2px 0 color-mix(in srgb, var(--color-accent, var(--primary)) 45%, transparent);
  }
</style>
