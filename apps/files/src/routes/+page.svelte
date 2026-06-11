<script lang="ts">
  /// The browser surface: tab strip, toolbar, the shared FileBrowser
  /// on the active tab's controller, status line — plus the FM-only
  /// operations layer: context menu, clipboard, rename, trash and the
  /// permanent-delete confirmation.
  import { onMount, tick } from "svelte";
  import { get, writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    FileBrowser,
    joinPath,
    type FileEntry,
    type Place,
  } from "@arlen/ui-kit/components/browser";
  import { openPath } from "$lib/adapter";
  import { activeController, newTab, tabs } from "$lib/stores/tabs";
  import { loadPlaces } from "$lib/stores/places";
  import { pathEditing } from "$lib/stores/ui";
  import { clipboard, paste, runOp } from "$lib/stores/ops";
  import FmToolbar from "$lib/components/FmToolbar.svelte";
  import TabStrip from "$lib/components/TabStrip.svelte";
  import FmStatusBar from "$lib/components/FmStatusBar.svelte";
  import OpsOverlays from "$lib/components/OpsOverlays.svelte";

  const homePath = writable("/home");
  let selected = $state<FileEntry[]>([]);
  let renamingName = $state<string | null>(null);
  let confirmDelete = $state(false);

  // The visible listing, mirrored for the status line.
  let entries = $state<FileEntry[]>([]);
  $effect(() => {
    const c = $activeController;
    if (!c) return;
    return c.entries.subscribe((list) => (entries = list));
  });
  // A tab switch shows the new tab's selection state, which starts
  // empty — the browser republishes on interaction.
  $effect(() => {
    void $activeController;
    selected = [];
  });

  const currentPath = (): string => {
    const c = get(activeController);
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
    try {
      const places = await invoke<{ orte: Place[] }>("files_places");
      const home = places.orte.find((p) => p.icon === "home");
      if (home) homePath.set(home.path);
      if ($tabs.length === 0) newTab(home?.path ?? "/home");
    } catch {
      if ($tabs.length === 0) newTab("/home");
    }
    await loadPlaces();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="fm" onkeydown={onOpsKeydown}>
  <TabStrip />
  {#if $activeController}
    <FmToolbar
      controller={$activeController}
      homePath={$homePath}
      bind:pathEditing={$pathEditing}
    />
    <ContextMenu.Root>
      <ContextMenu.Trigger class="fm-browse">
        <FileBrowser
          controller={$activeController}
          bind:renamingName
          onactivate={(entry, path) => {
            if (entry.kind !== "directory") void openPath(path);
          }}
          onselection={(list) => (selected = list)}
          onrenamecommit={(entry, newName) =>
            runOp("rename", [joinPath(currentPath(), entry.name)], newName)}
        />
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
        <ContextMenu.Separator />
        <ContextMenu.Item onclick={newFolder}>New folder</ContextMenu.Item>
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
    <FmStatusBar {entries} {selected} />
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
</style>
