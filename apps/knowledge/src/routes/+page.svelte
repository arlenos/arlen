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
  import { SidebarProvider, SidebarInset } from "@arlen/ui-kit/components/ui/sidebar";
  import TimelineView from "$lib/components/TimelineView.svelte";
  import ProjectsView from "$lib/components/ProjectsView.svelte";
  import SearchView from "$lib/components/SearchView.svelte";
  import LibraryView from "$lib/components/LibraryView.svelte";
  import { onMount } from "svelte";
  import { knowledgeAdapter, mocked, unavailable } from "$lib/adapter";
  import { labelKeyFor, emptyKeyFor } from "$lib/locations";
  import { days, flatEvents, loadTimeline, type TimelineEvent } from "$lib/stores/timeline";
  import { asOf, projectInfo, type ProjectInfo } from "$lib/stores/projects";
  import { query as searchQuery, loadSavedSearches, type SearchResult } from "$lib/stores/search";
  import type { LibraryEntry } from "$lib/stores/library";
  import { initAppMenu } from "$lib/menu";
  import { t } from "$lib/i18n/messages";

  onMount(() => {
    void initAppMenu();
    // The projects detail reuses the timeline's events for its recent-activity
    // block, so both fixtures stay one story.
    void loadTimeline();
    // The Searches place opens with what is on disk; it used to open with four
    // searches nobody had saved.
    void loadSavedSearches();
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

  function openPrivacySettings(): void {
    // The capability browser and the capsule list both live in Settings/Privacy
    // (decision 6); this links out rather than re-hosting either, by spawning
    // `arlen-settings --panel`.
    settingsOpenFailed = false;
    void invoke("open_settings_route", { route: "/privacy" }).catch(() => {
      // A link to a privacy control that silently does nothing is the worst
      // shape for this affordance: the person clicking it concludes there is no
      // such control.
      settingsOpenFailed = true;
    });
  }

  const now = Date.now();

  /// Whether the search surface owns the content area, which it does whenever
  /// there is a query - from anywhere - and at rest in the Searches place.
  ///
  /// Named once because the heading and the content have to agree. They did not:
  /// the title read `labelKeyFor($path)` while the region under it rendered
  /// `SearchView`, so searching from the timeline put "Timeline" above a list of
  /// matches, and above "Nothing matches" when there were none. The app already
  /// holds the right answer for this - `basePlaceId` resolves a `search:` location
  /// to the Searches place for exactly this label - and typing in the titlebar was
  /// the one route into the search surface that went around it.
  const searchOwnsContent = $derived($searchQuery.trim().length > 0 || $path === "searches");
</script>

<SidebarProvider class="h-screen min-h-0 overflow-hidden">
  <KnowledgeSidebar activeLocation={$path} onnavigate={navigate} onsettings={openPrivacySettings} />
  <SidebarInset class="h-svh min-h-0">
    <!-- The bar carries the place (the files canon); the in-content heading is
         gone - one context, said once. -->
    <KnowledgeHeader placeLabel={$t(labelKeyFor(searchOwnsContent ? "searches" : $path))} />
    <div class="kn-body">
    <main class="kn-main">
    {#if settingsOpenFailed}
      <!-- The capability browser lives in Settings; if it would not start, say
           so rather than leaving the click looking like there is no such page. -->
      <p class="kn-open-failed" role="alert">{$t("k.settingsOpenFailed")}</p>
    {/if}
    {#if searchOwnsContent}
      <!-- The titlebar query owns the content area wherever you are; the
           Searches place shows the same surface at rest (the saved list). -->
      <SearchView onselect={onSearchSelect} />
    {:else if $path === "timeline"}
      <TimelineView onselect={(e) => (selectedEvent = e)} />
    {:else if $path === "projects"}
      <ProjectsView onselect={onProjectsSelect} />
    {:else if $path === "library"}
      <LibraryView onselect={onLibrarySelect} />
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
          errorTitle={$t("k.fb.errorTitle")}
          hintUnknown={$t("k.fb.hintUnknown")}
          browserLabel={$t("k.fb.browserLabel")}
          hintPermission={$t("k.fb.hintPermission")}
          hintNotConnected={$t("k.fb.hintNotConnected")}
          hintNoSuchDir={$t("k.fb.hintNoSuchDir")}
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
  </SidebarInset>
</SidebarProvider>

<style>
  /* A failure, coloured like one.
   *
   * This was 62% of the foreground - DIMMER than the body text beside it - for a
   * line that only appears because somebody clicked Capabilities and no window
   * opened. Rendered next to the place heading it read as a subtitle. The
   * meetings capture screen already paints its equivalent `var(--color-error)`;
   * one convention across the apps, and the thing the person has to notice is
   * not the quietest text on screen. */
  .kn-open-failed {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--color-error, #f87171);
  }

  .kn-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .kn-main {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .kn-browser {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
