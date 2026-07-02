<script lang="ts">
  /// The browser surface: the shared FileBrowser on the active tab's
  /// controller, the status line — plus the FM-only operations layer:
  /// context menu, clipboard, rename, trash and the permanent-delete
  /// confirmation. The chrome (nav, breadcrumb, tabs, toggles) lives
  /// in the layout's headerbar.
  import { onMount, onDestroy, tick } from "svelte";
  import { get, writable } from "svelte/store";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { tauriAvailable } from "$lib/tauri";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { AboutDialog } from "@arlen/ui-kit/components/ui/about-dialog";
  import {
    FileBrowser,
    joinPath,
    parentPath,
    isVirtualLocation,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { invoke } from "@tauri-apps/api/core";
  import { openPath } from "$lib/adapter";
  import { restoreFromTrash, emptyTrash, deletePermanently } from "$lib/stores/trash";
  import { isArchiveName } from "$lib/archive";
  import { activeController, newTab, tabs } from "$lib/stores/tabs";
  import { focusedController, focusedPane, paneB, splitView } from "$lib/stores/panes";
  import { addBookmark, homePath, loadPlaces } from "$lib/stores/places";
  import { infoOpen } from "$lib/stores/ui";
  import { clipboard, paste, runOp, bulkRename, extractArchive, compressPaths } from "$lib/stores/ops";
  import FmStatusBar from "$lib/components/FmStatusBar.svelte";
  import OpsOverlays from "$lib/components/OpsOverlays.svelte";
  import FmBatchRename from "$lib/components/FmBatchRename.svelte";
  import FmSearchBar from "$lib/components/FmSearchBar.svelte";
  import FmSearchResults from "$lib/components/FmSearchResults.svelte";
  import FmDuplicates from "$lib/components/FmDuplicates.svelte";
  import FmFacetBar from "$lib/components/FmFacetBar.svelte";
  import FmAskBanner from "$lib/components/FmAskBanner.svelte";
  import FmInfoPanel from "$lib/components/FmInfoPanel.svelte";
  import { savedSearches } from "$lib/stores/places";
  import { searchOpen, searchResults } from "$lib/stores/search";
  import {
    facetOpen,
    facetBase,
    loadFacetOptions,
    loadSmartFolders,
    serializeFacets,
    selectedFacets,
    clearFacets,
  } from "$lib/stores/facets";
  import { duplicatesOpen } from "$lib/stores/duplicates";
  import { askDraft, runAsk, applyDraft, clearAsk, loadAiEnabled } from "$lib/stores/ask";
  import { columnsFor, emptyLabelFor } from "$lib/locations";
  import { DEFAULT_COLUMNS } from "@arlen/ui-kit/components/browser";

  let renamingName = $state<string | null>(null);
  let batchRenaming = $state(false);
  let aboutOpen = $state(false);
  let confirmDelete = $state(false);

  // Each pane's columns + empty message follow its own location (a virtual
  // location swaps Size for the item's home folder), live as the pane navigates.
  let aColumns = $state(DEFAULT_COLUMNS);
  let aEmpty = $state("This folder is empty");
  $effect(() => {
    const c = $activeController;
    if (!c) return;
    return c.path.subscribe((p) => {
      aColumns = columnsFor(p);
      aEmpty = emptyLabelFor(p);
    });
  });
  let bColumns = $state(DEFAULT_COLUMNS);
  let bEmpty = $state("This folder is empty");
  $effect(() => {
    const c = $paneB;
    if (!c) return;
    return c.path.subscribe((p) => {
      bColumns = columnsFor(p);
      bEmpty = emptyLabelFor(p);
    });
  });

  // The focused pane's location, live, gates the virtual-location actions.
  let focusedPath = $state("/");
  $effect(() => {
    const c = $focusedController;
    if (!c) return;
    return c.path.subscribe((p) => (focusedPath = p));
  });
  const isVirtual = $derived(isVirtualLocation(focusedPath));
  const isTrash = $derived(focusedPath === "trash");
  let confirmEmpty = $state(false);

  // When the filter bar opens, remember the real folder it opened over so
  // clearing every facet returns there (not to a stale facet: location).
  $effect(() => {
    if ($facetOpen) {
      const p = currentPath();
      if (!p.startsWith("facet:")) facetBase.set(p);
    }
  });

  // A virtual location defaults to newest-first by its time column (Last
  // accessed / Deleted), set once per arrival so a later re-sort sticks.
  let lastSortDefaulted = "";
  $effect(() => {
    const c = $focusedController;
    const p = focusedPath;
    if (!c) return;
    if (isVirtualLocation(p) && p !== lastSortDefaulted) {
      lastSortDefaulted = p;
      void c.setSort("modified", false);
    } else if (!isVirtualLocation(p)) {
      lastSortDefaulted = "";
    }
  });

  /// Open every selected virtual-location entry from its real home path
  /// (Recent / project / search items live in different folders).
  async function openVirtualSelection(): Promise<void> {
    for (const e of selected) if (e.full_path) await openPath(e.full_path);
  }

  /// The unifying bridge back to the filesystem: navigate to the (first)
  /// selected item's containing folder, wherever the item really lives.
  function goToFolder(): void {
    const e = selected[0];
    if (!e?.full_path) return;
    void get(focusedController)?.navigate(parentPath(e.full_path) ?? e.full_path);
  }

  /// Restore every selected trashed entry to its recorded original location,
  /// then refresh the trash listing.
  async function restoreSelection(): Promise<void> {
    for (const e of selected) await restoreFromTrash(e);
    await get(focusedController)?.refresh();
  }

  /// Permanently delete every selected trashed entry (bypassing restore), then
  /// refresh the trash listing.
  async function deleteSelectionPermanently(): Promise<void> {
    for (const e of selected) await deletePermanently(e);
    await get(focusedController)?.refresh();
  }

  /// Extract the selected archive into the current folder.
  async function extractSelection(): Promise<void> {
    const e = selected[0];
    if (!e) return;
    await extractArchive(joinPath(currentPath(), e.name), currentPath());
  }

  /// Compress the selection into a new archive in the current folder. One item
  /// keeps its name; several become "Archive.zip".
  async function compressSelection(): Promise<void> {
    if (selected.length === 0) return;
    const name = selected.length === 1 ? `${selected[0]?.name}.zip` : "Archive.zip";
    await compressPaths(selectedPaths(), joinPath(currentPath(), name));
  }

  /// Permanently empty the trash, then refresh.
  async function doEmptyTrash(): Promise<void> {
    await emptyTrash();
    confirmEmpty = false;
    await get(focusedController)?.refresh();
  }

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

  /// "Ask Arlen": run the scoped natural-language ask, adopt the drafted facets
  /// into the live filter (the user then sees the chips + the listing as a
  /// preview), and navigate to the result. A failed draft leaves the bar as is.
  async function askArlen(query: string) {
    const folder = currentPath();
    const result = await runAsk(folder, query);
    if (!result) return;
    applyDraft(result, query);
    const loc = serializeFacets(get(selectedFacets)) || folder;
    await get(focusedController)?.navigate(loc);
  }

  /// Dismiss the drafted filter: drop the banner + the facets and return to the
  /// folder the ask ran over.
  function dismissAsk() {
    clearAsk();
    clearFacets();
    void get(focusedController)?.navigate(get(facetBase));
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

  /// Create a symbolic link in the current folder pointing at the single
  /// selected entry (the backend `files_symlink`; the FM root capability
  /// confines it). The link is named "Link to <name>"; refresh so it shows.
  async function createLink() {
    if (selected.length !== 1) return;
    const name = `Link to ${selected[0].name}`;
    const target = joinPath(currentPath(), selected[0].name);
    try {
      await invoke("files_symlink", { parent: currentPath(), name, target });
      await get(focusedController)?.refresh();
    } catch (err) {
      console.warn("files: create link failed", err);
    }
  }

  /// The "Open With" submenu's apps for the selected file (the apps that declare
  /// its MIME type, from `files_apps_for`). null = not loaded yet; loaded lazily
  /// when the submenu opens so a right-click does not pay an xdg-mime + dir scan.
  interface OpenWithApp {
    name: string;
    exec: string;
    terminal: boolean;
  }
  let openWithApps = $state<OpenWithApp[] | null>(null);

  async function loadOpenWith() {
    if (selected.length !== 1) return;
    openWithApps = null;
    const p = joinPath(currentPath(), selected[0].name);
    try {
      openWithApps = await invoke<OpenWithApp[]>("files_apps_for", { path: p });
    } catch {
      openWithApps = [];
    }
  }

  /// Open the selected file with the chosen app's `.desktop` Exec (the backend
  /// expands the field codes + spawns without a shell).
  async function openWith(exec: string) {
    if (selected.length !== 1) return;
    const p = joinPath(currentPath(), selected[0].name);
    try {
      await invoke("files_open_with", { path: p, exec });
    } catch (err) {
      console.warn("files: open-with failed", err);
    }
  }

  /// The user's `~/Templates` entries (the backend `files_templates`), offered
  /// in the context menu's "New from template" submenu.
  interface Template {
    label: string;
    icon: string;
    path: string;
  }
  const templates = writable<Template[]>([]);

  /// Create a new file in the current folder from `t` by copying the template
  /// (the existing copy op, so no separate command), then start an inline
  /// rename on the new file - the same flow as New folder.
  async function newFromTemplate(t: Template) {
    const ok = await runOp("copy", [t.path], currentPath());
    if (ok) {
      await tick();
      renamingName = t.label;
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

  /// Reverse the last file operation (Ctrl+Z). `files_undo` pops the op log and
  /// inverts the most recent op (the backend `UndoStack`), returning whether
  /// anything was undone; refresh the focused pane so the reversal shows.
  /// Best-effort: an undo error leaves the listing unchanged.
  async function undo() {
    if (!tauriAvailable) return;
    try {
      const undone = await invoke<boolean>("files_undo");
      if (undone) await get(focusedController)?.refresh();
    } catch (err) {
      console.warn("files: undo failed", err);
    }
  }

  // The topbar menu lives in the shell; a click there travels back over
  // the Event Bus and the host forwards it as `arlen://menu-action`.
  let unlistenMenu: UnlistenFn | null = null;

  /// Run a topbar-menu action. Action ids mirror the menu published in
  /// `publish_app_menu` (src-tauri/src/lib.rs); each maps to the same
  /// operation its keyboard/context-menu equivalent runs.
  async function runMenuAction(action: string) {
    const c = get(focusedController);
    switch (action) {
      case "file.new_folder":
        await newFolder();
        break;
      case "file.new_window":
        // A separate top-level window is a follow-up; open a new tab at
        // the current location so the item is not inert.
        newTab(currentPath());
        break;
      case "file.properties":
        infoOpen.set(true);
        break;
      case "file.close":
        if (tauriAvailable) await getCurrentWindow().close();
        break;
      case "edit.undo":
        await undo();
        break;
      case "edit.cut":
        copySelection("move");
        break;
      case "edit.copy":
        copySelection("copy");
        break;
      case "edit.paste":
        await paste(currentPath());
        break;
      case "edit.rename":
        if (selected.length === 1) renamingName = selected[0].name;
        break;
      case "edit.trash":
        await trashSelection();
        break;
      case "view.refresh":
        await c?.refresh();
        break;
      case "view.toggle_hidden":
        if (c) await c.setShowHidden(!get(c.showHidden));
        break;
      case "view.sort.name":
        await c?.setSort("name");
        break;
      case "view.sort.size":
        await c?.setSort("size");
        break;
      case "view.sort.type":
        await c?.setSort("type");
        break;
      case "view.sort.modified":
        await c?.setSort("modified");
        break;
      case "go.home":
        await get(activeController)?.navigate(get(homePath));
        break;
      case "go.up":
        await c?.up();
        break;
      case "go.trash":
        await get(activeController)?.navigate("trash");
        break;
      case "go.recent":
        await get(activeController)?.navigate("recent");
        break;
      case "edit.select_all":
        c?.selectAll();
        break;
      case "help.about":
        aboutOpen = true;
        break;
      default:
        console.info(`files: topbar menu action not yet wired: ${action}`);
    }
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
    } else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z") {
      // Ctrl+Shift+Z is reserved for a future redo, so guard on !shiftKey.
      e.preventDefault();
      void undo();
    } else if (e.key === "F2" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // F2 starts an inline rename on a single selected item (the menu's edit.rename).
      if (selected.length === 1) {
        e.preventDefault();
        renamingName = selected[0].name;
      }
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      // Ctrl+A selects everything in the focused pane (the menu's edit.select_all);
      // preventDefault so it does not select the page text instead.
      e.preventDefault();
      get(focusedController)?.selectAll();
    }
  }

  const deleteMessage = $derived(
    selected.length === 1
      ? `Delete ${selected[0]?.name} forever? This cannot be undone.`
      : `Delete ${selected.length} items forever? This cannot be undone.`,
  );

  onMount(async () => {
    await loadPlaces();
    void loadFacetOptions();
    void loadSmartFolders();
    void loadAiEnabled();
    if (get(tabs).length === 0) newTab(get(homePath));
    if (tauriAvailable) {
      invoke<Template[]>("files_templates")
        .then((t) => templates.set(t))
        .catch(() => {});
      unlistenMenu = await listen<{ action: string }>(
        "arlen://menu-action",
        (e) => void runMenuAction(e.payload.action),
      );
    }
  });
  onDestroy(() => unlistenMenu?.());
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="fm" onkeydown={onOpsKeydown}>
  {#if $activeController && $focusedController}
    <FmSearchBar path={currentPath()} onsave={saveSearch} onask={askArlen} />
    {#if $facetOpen}
      {#if $askDraft}
        <FmAskBanner scope={$facetBase} ondismiss={dismissAsk} />
      {/if}
      <FmFacetBar
        basePath={$facetBase}
        onnavigate={(loc) => $focusedController?.navigate(loc)}
      />
    {/if}
    <ContextMenu.Root>
      <ContextMenu.Trigger class="fm-browse">
        {#if $duplicatesOpen}
          <FmDuplicates
            ontrash={async (paths) => {
              await runOp("trash", paths);
              await get(focusedController)?.refresh();
            }}
          />
        {:else if $searchOpen && $searchResults !== null}
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
              columns={aColumns}
              emptyLabel={aEmpty}
              bind:renamingName
              onactivate={(entry, path) => {
                if (entry.kind === "directory") return;
                // An archive opens as a folder (browse its contents); other
                // files open with the system handler.
                if (isArchiveName(entry.name)) void $activeController?.navigate(path);
                else void openPath(path);
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
                columns={bColumns}
                emptyLabel={bEmpty}
                onactivate={(entry, path) => {
                  if (entry.kind === "directory") return;
                  if (isArchiveName(entry.name)) void $paneB?.navigate(path);
                  else void openPath(path);
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
              onnavigate={(loc) => $focusedController?.navigate(loc)}
            />
          {/if}
        </div>
        {/if}
      </ContextMenu.Trigger>
      <ContextMenu.Content class="w-52">
        {#if isVirtual}
          {#if selected.length > 0}
            {#if isTrash}
              <ContextMenu.Item onclick={restoreSelection}>Restore</ContextMenu.Item>
              <ContextMenu.Item variant="destructive" onclick={() => (confirmDelete = true)}>
                Delete permanently
              </ContextMenu.Item>
            {:else}
              <ContextMenu.Item onclick={openVirtualSelection}>Open</ContextMenu.Item>
            {/if}
            {#if selected.length === 1}
              <ContextMenu.Item onclick={goToFolder}>Go to folder</ContextMenu.Item>
            {/if}
          {/if}
          {#if isTrash}
            {#if selected.length > 0}<ContextMenu.Separator />{/if}
            <ContextMenu.Item variant="destructive" onclick={() => (confirmEmpty = true)}>
              Empty Trash
            </ContextMenu.Item>
          {/if}
        {:else}
        {#if selected.length > 0}
          <ContextMenu.Item
            onclick={() => {
              const e = selected[0];
              if (!e) return;
              const p = joinPath(currentPath(), e.name);
              if (e.kind === "directory" || isArchiveName(e.name))
                void $activeController?.navigate(p);
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
            <ContextMenu.Item onclick={() => void createLink()}>
              Create Link
            </ContextMenu.Item>
            {#if selected[0].kind !== "directory"}
              <ContextMenu.Sub
                onOpenChange={(open) => {
                  if (open) void loadOpenWith();
                }}
              >
                <ContextMenu.SubTrigger>Open With</ContextMenu.SubTrigger>
                <ContextMenu.SubContent class="w-52">
                  {#if openWithApps === null}
                    <ContextMenu.Item disabled>Loading…</ContextMenu.Item>
                  {:else if openWithApps.length === 0}
                    <ContextMenu.Item disabled>No apps found</ContextMenu.Item>
                  {:else}
                    {#each openWithApps as app (app.exec)}
                      <ContextMenu.Item onclick={() => void openWith(app.exec)}>
                        {app.name}
                      </ContextMenu.Item>
                    {/each}
                  {/if}
                </ContextMenu.SubContent>
              </ContextMenu.Sub>
            {/if}
          {/if}
          {#if selected.length > 1}
            <ContextMenu.Item onclick={() => (batchRenaming = true)}>
              Rename&hellip;
            </ContextMenu.Item>
          {/if}
        {/if}
        {#if selected.length === 0}
          <ContextMenu.Separator />
          <ContextMenu.Item onclick={newFolder}>New folder</ContextMenu.Item>
          {#if $templates.length > 0}
            <ContextMenu.Sub>
              <ContextMenu.SubTrigger>New from template</ContextMenu.SubTrigger>
              <ContextMenu.SubContent class="w-52">
                {#each $templates as t (t.path)}
                  <ContextMenu.Item onclick={() => void newFromTemplate(t)}>
                    {t.label}
                  </ContextMenu.Item>
                {/each}
              </ContextMenu.SubContent>
            </ContextMenu.Sub>
          {/if}
        {/if}
        {#if selected.length > 0}
          <ContextMenu.Separator />
          {#if selected.length === 1 && isArchiveName(selected[0]?.name ?? "")}
            <ContextMenu.Item onclick={extractSelection}>Extract here</ContextMenu.Item>
          {/if}
          <ContextMenu.Item onclick={compressSelection}>Compress to archive</ContextMenu.Item>
          <ContextMenu.Item onclick={trashSelection}>
            Move to trash
            <ContextMenu.Shortcut>Del</ContextMenu.Shortcut>
          </ContextMenu.Item>
          <ContextMenu.Item variant="destructive" onclick={() => (confirmDelete = true)}>
            Delete forever
          </ContextMenu.Item>
        {/if}
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
    if (isTrash) await deleteSelectionPermanently();
    else await runOp("delete", selectedPaths());
    confirmDelete = false;
  }}
  onCancel={() => (confirmDelete = false)}
/>

<ConfirmDialog
  open={confirmEmpty}
  title="Empty Trash"
  message="Permanently delete everything in the trash? This cannot be undone."
  confirmLabel="Empty Trash"
  variant="destructive"
  onConfirm={doEmptyTrash}
  onCancel={() => (confirmEmpty = false)}
/>

<FmBatchRename
  open={batchRenaming}
  names={selected.map((e) => e.name)}
  onClose={() => (batchRenaming = false)}
  onApply={(rule) => {
    batchRenaming = false;
    void bulkRename(currentPath(), selected.map((e) => e.name), rule);
  }}
/>

<AboutDialog
  open={aboutOpen}
  onClose={() => (aboutOpen = false)}
  appName="Files"
  version="0.1.0"
  description="Browse, organise and search your files."
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
