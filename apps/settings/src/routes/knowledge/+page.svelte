<script lang="ts">
  /// Knowledge Graph settings page.
  ///
  /// Built on the Arlen Design System canon (docs/architecture/design-system.md):
  /// Page + SectionGrid + Group + Row + ChipList are imported directly from
  /// `@arlen/ui-kit` (single source, no copy). Button + NumberInput are still
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
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { t } from "$lib/i18n/messages";
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
    if (bytes === null) return $t("s.know.unknown");
    if (bytes < 1024) return `${bytes} B`;
    const kb = bytes / 1024;
    if (kb < 1024) return `${kb.toFixed(1)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(2)} GB`;
  }
</script>

<Page
  title={$t("s.know.title")}
  description={$t("s.know.desc")}
>
  <SectionGrid>
    <Group label={$t("s.know.status")}>
      {#if error}
        <Row label={$t("s.know.statsUnavail")} description={$t("s.know.statsUnavail.desc")} id="kg-error">
          {#snippet control()}
            <span title={error}><AlertCircle size={16} class="kg-error-icon" /></span>
          {/snippet}
        </Row>
      {:else if stats}
        {@const s = stats}
        <Row
          label={$t("s.know.service")}
          description={s.daemonRunning
            ? $t("s.know.service.running")
            : $t("s.know.service.stopped")}
          id="kg-daemon-status"
        >
          {#snippet control()}
            <span class="meta" class:on={s.daemonRunning}>
              {s.daemonRunning ? $t("s.know.running") : $t("s.know.stopped")}
            </span>
          {/snippet}
        </Row>
        {#if s.daemonRunning}
          <Row label={$t("s.know.dbSize")} description={$t("s.know.dbSize.desc")} id="kg-db-size">
            {#snippet control()}
              <span class="meta"><Database size={12} strokeWidth={1.5} />{formatBytes(s.dbSizeBytes)}</span>
            {/snippet}
          </Row>
          <Row label={$t("s.know.graphSize")} description={$t("s.know.graphSize.desc")} id="kg-graph-size">
            {#snippet control()}
              <span class="meta"><HardDrive size={12} strokeWidth={1.5} />{formatBytes(s.graphSizeBytes)}</span>
            {/snippet}
          </Row>
          <Row
            label={$t("s.know.fuse")}
            description={s.fuseMounted ? $t("s.know.fuse.mounted") : $t("s.know.fuse.notMounted")}
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
      <Row label={$t("s.know.refresh")} description={$t("s.know.refresh.desc")} id="kg-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={loading} onclick={refresh}>
            <RefreshCw size={14} class={loading ? "kg-spin" : ""} />
            {$t("s.know.refresh")}
          </Button>
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.know.projectDetection")}>
      <Row
        label={$t("s.know.watchDirs")}
        description={watchDirs.length === 0
          ? $t("s.know.watchDirs.default", { dirs: PROJECTS_DEFAULTS.watch_directories.join(", ") })
          : $t("s.know.watchDirs.set")}
        id="kg-watch-dirs"
      >
        {#snippet below()}
          <ChipList
            bind:items={watchDirs}
            placeholder={$t("s.know.watchDirs.placeholder")}
            onchange={persistWatchDirs}
          />
        {/snippet}
      </Row>
      <Row
        label={$t("s.know.depth")}
        description={$t("s.know.depth.desc")}
        id="kg-max-depth"
      >
        {#snippet control()}
          <NumberInput width="var(--width-row-control, 200px)" value={maxDepth} min={1} max={10} onchange={setMaxDepth} />
        {/snippet}
      </Row>
      <Row
        label={$t("s.know.promote")}
        description={$t("s.know.promote.desc")}
        id="kg-promote"
      >
        {#snippet control()}
          <NumberInput width="var(--width-row-control, 200px)" value={promoteThreshold} min={1} max={20} onchange={setPromoteThreshold} />
        {/snippet}
      </Row>
      {#if projectDirty}
        <Row
          label={$t("s.know.restart")}
          description={$t("s.know.restart.desc")}
          id="kg-restart-hint"
        >
          {#snippet control()}
            <AlertCircle size={16} class="kg-warn-icon" />
          {/snippet}
        </Row>
      {/if}
    </Group>

    <Group label={$t("s.know.timeline")}>
      <Row
        label={$t("s.know.timelineFs")}
        description={$t("s.know.timelineFs.desc")}
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

    <Group label={$t("s.know.app")}>
      <Row
        label={$t("s.know.browse")}
        description={$t("s.know.browse.desc")}
        id="kg-app-link"
      >
        {#snippet control()}
          <Button variant="outline" size="sm" disabled>
            <Brain size={14} />
            {$t("s.know.openApp")}
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
    font-size: var(--text-sm);
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
    color: var(--color-warning);
  }
</style>
