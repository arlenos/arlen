<script lang="ts">
  /// Knowledge Graph settings page.
  ///
  /// Built on the Lunaris Design System canon (docs/architecture/design-system.md):
  /// Page + SectionGrid + Group + Row + ChipList are imported directly from
  /// `@lunaris/ui-kit` (single source, no copy). Button + NumberInput are still
  /// the app-local copies (Tailwind/lucide components, pending the S-U1b @source
  /// consolidation).
  ///
  /// Surfaces: read-only daemon/storage stats, the `graph.toml [projects]`
  /// project-detection config (watch dirs, recursion depth, auto-promote
  /// threshold), the FUSE timeline mount, and a disabled link to the Phase-8
  /// Knowledge app.

  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import {
    Brain,
    Database,
    FolderTree,
    HardDrive,
    RefreshCw,
    AlertCircle,
    FolderClock,
  } from "lucide-svelte";
  import { Page } from "@lunaris/ui-kit/components/ui/page";
  import { SectionGrid } from "@lunaris/ui-kit/components/ui/section-grid";
  import { Group } from "@lunaris/ui-kit/components/ui/group";
  import { Row } from "@lunaris/ui-kit/components/ui/row";
  import { ChipList } from "@lunaris/ui-kit/components/ui/chip-list";
  import { Button } from "@lunaris/ui-kit/components/ui/button";
  import { NumberInput } from "@lunaris/ui-kit/components/ui/number-input";
  import { graph, PROJECTS_DEFAULTS } from "$lib/stores/projectsConfig";

  interface KnowledgeStats {
    daemonRunning: boolean;
    fuseMount: string;
    fuseMounted: boolean;
    dbSizeBytes: number | null;
    graphSizeBytes: number | null;
  }

  let stats = $state<KnowledgeStats | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  // Explicit watch-directory override list (empty = using built-in defaults).
  let watchDirs = $state<string[]>([]);
  // Project-detection changes need a daemon restart (startup-only config); the
  // hint shows after a real edit this session.
  let projectDirty = $state(false);

  const maxDepth = $derived<number>(
    ($graph.data?.projects?.max_depth as number | undefined) ??
      PROJECTS_DEFAULTS.max_depth,
  );
  const promoteThreshold = $derived<number>(
    ($graph.data?.projects?.auto_promote_threshold as number | undefined) ??
      PROJECTS_DEFAULTS.auto_promote_threshold,
  );

  async function refresh() {
    loading = true;
    error = null;
    try {
      stats = await invoke<KnowledgeStats>("knowledge_stats_get");
    } catch (e) {
      error = String(e);
      stats = null;
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    await Promise.all([refresh(), graph.load()]);
    watchDirs =
      (get(graph).data?.projects?.watch_directories as string[] | undefined) ?? [];
  });

  async function persistWatchDirs() {
    projectDirty = true;
    await graph.setValue("projects.watch_directories", watchDirs);
  }

  async function setMaxDepth(v: number) {
    projectDirty = true;
    await graph.setValue("projects.max_depth", v);
  }

  async function setPromoteThreshold(v: number) {
    projectDirty = true;
    await graph.setValue("projects.auto_promote_threshold", v);
  }

  function formatBytes(bytes: number | null): string {
    if (bytes === null) return "—";
    if (bytes < 1024) return `${bytes} B`;
    const kb = bytes / 1024;
    if (kb < 1024) return `${kb.toFixed(1)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(2)} GB`;
  }
</script>

<Page
  title="Knowledge Graph"
  description="Lunaris keeps a private graph of the files, projects, and apps you use. Configure project detection here; browse the graph in the Knowledge app (Phase 8)."
>
  <SectionGrid>
    <Group label="Status">
      {#if error}
        <Row label="Stats unavailable" description={error} id="kg-error">
          {#snippet control()}
            <AlertCircle size={16} class="kg-error-icon" />
          {/snippet}
        </Row>
      {:else if stats}
        {@const s = stats}
        <Row
          label="Knowledge Daemon"
          description={s.daemonRunning
            ? "Running — stats are live."
            : "Not running — start the daemon to populate stats."}
          id="kg-daemon-status"
        >
          {#snippet control()}
            <span class="meta" class:on={s.daemonRunning}>
              {s.daemonRunning ? "Running" : "Stopped"}
            </span>
          {/snippet}
        </Row>
        {#if s.daemonRunning}
          <Row label="Database size" description="SQLite event store." id="kg-db-size">
            {#snippet control()}
              <span class="meta"><Database size={12} strokeWidth={1.5} />{formatBytes(s.dbSizeBytes)}</span>
            {/snippet}
          </Row>
          <Row label="Graph size" description="Ladybug graph storage on disk." id="kg-graph-size">
            {#snippet control()}
              <span class="meta"><HardDrive size={12} strokeWidth={1.5} />{formatBytes(s.graphSizeBytes)}</span>
            {/snippet}
          </Row>
          <Row
            label="FUSE mount"
            description={s.fuseMounted ? "Browseable as a filesystem." : "Not mounted."}
            id="kg-fuse-mount"
          >
            {#snippet control()}
              <span class="meta" class:on={s.fuseMounted}>
                <FolderTree size={12} strokeWidth={1.5} />{s.fuseMount}
              </span>
            {/snippet}
          </Row>
        {/if}
      {/if}
      <Row label="Refresh" description="Re-read filesystem stats." id="kg-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={loading} onclick={refresh}>
            <RefreshCw size={14} class={loading ? "kg-spin" : ""} />
            Refresh
          </Button>
        {/snippet}
      </Row>
    </Group>

    <Group label="Project Detection">
      <Row
        label="Watch directories"
        description={watchDirs.length === 0
          ? `Using defaults: ${PROJECTS_DEFAULTS.watch_directories.join(", ")}. Add a directory to override.`
          : "Directories scanned for projects. Restart the daemon to apply."}
        id="kg-watch-dirs"
      >
        {#snippet below()}
          <ChipList
            bind:items={watchDirs}
            placeholder="Add a directory, e.g. ~/Projects"
            onchange={persistWatchDirs}
          />
        {/snippet}
      </Row>
      <Row
        label="Recursion depth"
        description="How deep to scan each watch directory for projects."
        id="kg-max-depth"
      >
        {#snippet control()}
          <NumberInput value={maxDepth} min={1} max={10} onchange={setMaxDepth} />
        {/snippet}
      </Row>
      <Row
        label="Auto-promote threshold"
        description="Files opened in a session before an inferred project is promoted."
        id="kg-promote"
      >
        {#snippet control()}
          <NumberInput value={promoteThreshold} min={1} max={20} onchange={setPromoteThreshold} />
        {/snippet}
      </Row>
      {#if projectDirty}
        <Row
          label="Restart required"
          description="The Knowledge Daemon reads this config at startup; restart it to apply project-detection changes."
          id="kg-restart-hint"
        >
          {#snippet control()}
            <AlertCircle size={16} class="kg-warn-icon" />
          {/snippet}
        </Row>
      {/if}
    </Group>

    <Group label="Timeline">
      <Row
        label="Timeline filesystem"
        description="The graph is browseable as files at the mount path."
        id="kg-timeline"
      >
        {#snippet control()}
          <span class="meta">
            <FolderClock size={12} strokeWidth={1.5} />
            {stats?.fuseMount ?? "~/.timeline"}
          </span>
        {/snippet}
      </Row>
    </Group>

    <Group label="Knowledge App">
      <Row
        label="Browse the graph"
        description="Timeline, projects, and semantic search across your files. Coming with Phase 8."
        id="kg-app-link"
      >
        {#snippet control()}
          <Button variant="outline" size="sm" disabled>
            <Brain size={14} />
            Open Knowledge App
          </Button>
        {/snippet}
      </Row>
    </Group>
  </SectionGrid>
</Page>

<style>
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .meta.on {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }

  :global(.kg-spin) {
    animation: kg-spin 0.8s linear infinite;
  }
  @keyframes kg-spin {
    to {
      transform: rotate(360deg);
    }
  }
  :global(.kg-error-icon) {
    color: var(--destructive);
  }
  :global(.kg-warn-icon) {
    color: #f59e0b;
  }
</style>
