<script lang="ts">
  /// The browser surface: tab strip, toolbar, the shared FileBrowser
  /// on the active tab's controller, status line. The first tab opens
  /// at Home on mount; places and the home path come from the host.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import {
    FileBrowser,
    type FileEntry,
    type Place,
  } from "@arlen/ui-kit/components/browser";
  import { openPath } from "$lib/adapter";
  import { activeController, newTab, tabs } from "$lib/stores/tabs";
  import { loadPlaces } from "$lib/stores/places";
  import { pathEditing } from "$lib/stores/ui";
  import FmToolbar from "$lib/components/FmToolbar.svelte";
  import TabStrip from "$lib/components/TabStrip.svelte";
  import FmStatusBar from "$lib/components/FmStatusBar.svelte";

  const homePath = writable("/home");
  let selected = $state<FileEntry[]>([]);

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

<div class="fm">
  <TabStrip />
  {#if $activeController}
    <FmToolbar
      controller={$activeController}
      homePath={$homePath}
      bind:pathEditing={$pathEditing}
    />
    <FileBrowser
      controller={$activeController}
      onactivate={(entry, path) => {
        if (entry.kind !== "directory") void openPath(path);
      }}
      onselection={(list) => (selected = list)}
    />
    <FmStatusBar {entries} {selected} />
  {/if}
</div>

<style>
  .fm {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
</style>
