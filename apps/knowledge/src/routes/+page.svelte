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
  import { onMount } from "svelte";
  import { knowledgeAdapter, mocked } from "$lib/adapter";
  import { labelKeyFor, emptyKeyFor } from "$lib/locations";
  import { days, flatEvents, loadTimeline, type TimelineEvent } from "$lib/stores/timeline";
  import { asOf, projectInfo, type ProjectInfo } from "$lib/stores/projects";
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

  function openCapabilities(): void {
    // The generic capability browser lives in Settings/Privacy (decision 6); this
    // links out rather than re-hosting it. Live: a cross-app open command (a coder
    // seam); under vite it is a no-op.
    void invoke("open_settings_route", { route: "/privacy" }).catch(() => {});
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
      <!-- Timeline and Projects carry their own example-data line. -->
      {#if $mocked && $path !== "timeline" && $path !== "projects"}<span class="kn-sample">{$t("k.sample")}</span>{/if}
    </header>
    {#if $path === "timeline"}
      <TimelineView onselect={(e) => (selectedEvent = e)} />
    {:else if $path === "projects"}
      <ProjectsView onselect={onProjectsSelect} />
    {:else}
      <div class="kn-browser">
        <FileBrowser
          controller={ctrl}
          {onactivate}
          {onselection}
          {now}
          nameLabel={$t(labelKeyFor($path))}
          emptyLabel={$t(emptyKeyFor($path))}
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
  .kn-sample {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .kn-browser {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
