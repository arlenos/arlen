<script lang="ts">
  /// The Knowledge browser shell (KA-R1): the places sidebar + the content area on
  /// the shared kit browser controller + an on-demand detail panel. Reuses
  /// `createBrowserState` + `FileBrowser` from `@arlen/ui-kit/components/browser`
  /// (the same shell the Files app rides), with the KG adapter in place of the file
  /// adapter. Fixture-backed under vite; the scoped KG reads are coder seams.
  import { invoke } from "@tauri-apps/api/core";
  import { createBrowserState, FileBrowser, type FileEntry } from "@arlen/ui-kit/components/browser";
  import KnowledgeHeader from "$lib/components/KnowledgeHeader.svelte";
  import KnowledgeSidebar from "$lib/components/KnowledgeSidebar.svelte";
  import KnowledgeDetail from "$lib/components/KnowledgeDetail.svelte";
  import TimelineView from "$lib/components/TimelineView.svelte";
  import ProjectsView from "$lib/components/ProjectsView.svelte";
  import SearchView from "$lib/components/SearchView.svelte";
  import LibraryView from "$lib/components/LibraryView.svelte";
  import CapsulesView from "$lib/components/CapsulesView.svelte";
  import { onMount } from "svelte";
  import { knowledgeAdapter, mocked, unavailable } from "$lib/adapter";
  import { labelKeyFor, emptyKeyFor } from "$lib/locations";
  import { days, flatEvents, loadTimeline, type TimelineEvent } from "$lib/stores/timeline";
  import { asOf, projectInfo, type ProjectInfo } from "$lib/stores/projects";
  import { query as searchQuery, type SearchResult } from "$lib/stores/search";
  import type { LibraryEntry } from "$lib/stores/library";
  import { initAppMenu } from "$lib/menu";
  import { t } from "$lib/i18n/messages";

  onMount(() => {
    void initAppMenu();
    // The projects detail reuses the timeline's events for its recent-activity
    // block, so both fixtures stay one story.
    void loadTimeline();
  });

  // The headless controller auto-loads its initial place (Timeline, the spine).
  const ctrl = createBrowserState(knowledgeAdapter, { initial: "timeline", allowVirtual: true });
  const path = ctrl.path;

  let selected = $state<FileEntry | null>(null);
  let selectedEvent = $state<TimelineEvent | null>(null);
  let selectedProject = $state<ProjectInfo | null>(null);

  function navigate(location: string): void {
    selected = null;
    selectedEvent = null;
    selectedProject = null;
    void ctrl.navigate(location);
  }

  // A library item opens the plain entry panel with its display shape.
  function onLibrarySelect(e: LibraryEntry): void {
    selectedProject = null;
    selectedEvent = null;
    selected = {
      name: e.title,
      kind: "file",
      size: null,
      modified_unix: e.at,
      is_hidden: false,
      readonly: true,
      symlink_target: null,
    };
  }

  // A search hit opens the plain entry panel with its display shape.
  function onSearchSelect(r: SearchResult): void {
    selectedProject = null;
    selectedEvent = null;
    selected = {
      name: r.title,
      kind: "file",
      size: null,
      modified_unix: r.at ?? null,
      is_hidden: false,
      readonly: true,
      symlink_target: null,
    };
  }

  // A projects selection is either a project (its info panel) or a deeper
  // node (the plain entry panel).
  function onProjectsSelect(entry: FileEntry | null, path: string): void {
    selectedEvent = null;
    const atProjectLevel = path.replace(/\/+$/, "") === "/projects";
    if (entry && atProjectLevel) {
      selectedProject = projectInfo(entry.name, $days ? flatEvents($days) : [], $asOf);
      selected = selectedProject ? null : entry;
    } else {
      selectedProject = null;
      selected = entry;
    }
  }
  function onselection(entries: FileEntry[]): void {
    selected = entries[0] ?? null;
  }
  function onactivate(entry: FileEntry): void {
    selected = entry;
  }

  /// True when the last attempt to open Settings did not start it.
  let settingsOpenFailed = $state(false);

  function openCapabilities(): void {
    // The generic capability browser lives in Settings/Privacy (decision 6); this
    // links out rather than re-hosting it, by spawning `arlen-settings --panel`.
    settingsOpenFailed = false;
    void invoke("open_settings_route", { route: "/privacy" }).catch(() => {
      // A link to a privacy control that silently does nothing is the worst
      // shape for this affordance: the person clicking it concludes there is no
      // such control.
      settingsOpenFailed = true;
    });
  }

  const now = Date.now();
</script>

<div class="kn-app">
  <KnowledgeHeader />
  <div class="kn">
    <KnowledgeSidebar activeLocation={$path} onnavigate={navigate} oncapabilities={openCapabilities} />

    <main class="kn-main">
    <header class="kn-head">
      <h1 class="kn-h1">{$t(labelKeyFor($path))}</h1>
      <!-- Every designed place carries its own example-data line. -->

      <!-- The capability browser lives in Settings; if it would not start, say
           so rather than leaving the click looking like there is no such page. -->
      {#if settingsOpenFailed}
        <p class="kn-open-failed" role="alert">{$t("k.settingsOpenFailed")}</p>
      {/if}
    </header>
    {#if $searchQuery.trim().length > 0 || $path === "searches"}
      <!-- The titlebar query owns the content area wherever you are; the
           Searches place shows the same surface at rest (the saved list). -->
      <SearchView onselect={onSearchSelect} />
    {:else if $path === "timeline"}
      <TimelineView onselect={(e) => (selectedEvent = e)} />
    {:else if $path === "projects"}
      <ProjectsView onselect={onProjectsSelect} />
    {:else if $path === "library"}
      <LibraryView onselect={onLibrarySelect} />
    {:else if $path === "capsules"}
      <CapsulesView />
    {:else}
      <div class="kn-browser">
        <!-- "nothing here" and "could not read" are the same empty browser, so
             the empty label has to say which one it is. -->
        <FileBrowser
          controller={ctrl}
          {onactivate}
          {onselection}
          {now}
          nameLabel={$t(labelKeyFor($path))}
          emptyLabel={$unavailable ? $t("k.browse.unavailable") : $t(emptyKeyFor($path))}
        />
      </div>
    {/if}
  </main>

    {#if selectedProject}
      <KnowledgeDetail project={selectedProject} onclose={() => (selectedProject = null)} />
    {:else if selectedEvent}
      <KnowledgeDetail event={selectedEvent} onclose={() => (selectedEvent = null)} />
    {:else if selected}
      <KnowledgeDetail entry={selected} onclose={() => (selected = null)} />
    {/if}
  </div>
</div>

<style>
  .kn-open-failed {
    margin: 0;
    font-size: 0.85rem;
    color: color-mix(in srgb, var(--color-fg-primary) 62%, transparent);
  }

  .kn-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app);
    color: var(--color-fg-primary);
  }
  .kn {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .kn-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .kn-head {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    padding: 0.9rem 1.1rem 0.6rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .kn-h1 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .kn-browser {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
