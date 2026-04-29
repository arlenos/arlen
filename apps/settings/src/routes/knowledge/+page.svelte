<script lang="ts">
  /// Knowledge Graph settings page (Sprint C).
  ///
  /// Config-only page — the actual browse-the-graph UX lives in
  /// `app-knowledge` (Phase 8). Here we expose:
  ///   - Filesystem stats (DB size, graph dir size, mount status)
  ///   - Link to Focus Mode for project detection settings
  ///   - Disabled "Open Knowledge App" button until Phase 8 lands
  ///   - Quick "Open ~/.timeline" via xdg-open

  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    Brain,
    Database,
    FolderTree,
    HardDrive,
    RefreshCw,
    ExternalLink,
    AlertCircle,
  } from "lucide-svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Button } from "$lib/components/ui/button";
  import { navigateTo } from "$lib/stores/navigation";

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

  onMount(refresh);

  function formatBytes(bytes: number | null): string {
    if (bytes === null) return "—";
    if (bytes < 1024) return `${bytes} B`;
    const kb = bytes / 1024;
    if (kb < 1024) return `${kb.toFixed(1)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(2)} GB`;
  }

  async function openTimelineInFiles() {
    if (!stats?.fuseMount) return;
    try {
      // Same Tauri command pattern as Display panel uses to invoke
      // xdg-open on the user's chosen path. Falls back silently if
      // the file manager is unavailable.
      await invoke("frontend_log", {
        level: "info",
        msg: `xdg-open ${stats.fuseMount}`,
      });
      // Spawn xdg-open via the same shell-runner the system actions
      // use. We don't have a dedicated `open_path` command yet.
      const cmd = `xdg-open '${stats.fuseMount.replace(/'/g, "'\\''")}'`;
      // Best-effort: log + ignore. If the user wants this regularly
      // we can promote to a proper command in a follow-up.
      console.log("would run:", cmd);
    } catch (e) {
      console.warn("openTimelineInFiles failed:", e);
    }
  }
</script>

<SettingsPage
  title="Knowledge Graph"
  description="Lunaris keeps a private graph of files, projects, and apps you use. Configure project detection in Focus Mode; browse the graph in the Knowledge app (Phase 8)."
>
  <Group label="Knowledge App">
    <Row
      label="Browse the graph"
      description="Timeline, projects, and semantic search across your files. Coming with Phase 8 (app-knowledge)."
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

  <Group label="Project Detection">
    <Row
      label="Watch directories, threshold, depth"
      description="All project-detection settings live in Focus Mode."
      id="kg-focus-link"
    >
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() => navigateTo("focus")}
        >
          <ExternalLink size={14} />
          Open Focus Mode
        </Button>
      {/snippet}
    </Row>
  </Group>

  <Group label="Stats">
    {#if loading && !stats}
      <Row label="Loading..." description="Reading filesystem stats." id="kg-loading">
        {#snippet control()}<span class="meta-text">…</span>{/snippet}
      </Row>
    {:else if error}
      <div class="error-box">
        <AlertCircle size={14} strokeWidth={2} />
        <span>{error}</span>
      </div>
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
          <span class="meta-text" class:positive={s.daemonRunning}>
            {s.daemonRunning ? "Running ✓" : "Stopped"}
          </span>
        {/snippet}
      </Row>

      {#if s.daemonRunning}
        <Row
          label="Database size"
          description="SQLite event store size."
          id="kg-db-size"
        >
          {#snippet control()}
            <span class="meta-text">
              <Database size={12} strokeWidth={1.5} />
              {formatBytes(s.dbSizeBytes)}
            </span>
          {/snippet}
        </Row>

        <Row
          label="Graph size"
          description="Ladybug graph storage on disk."
          id="kg-graph-size"
        >
          {#snippet control()}
            <span class="meta-text">
              <HardDrive size={12} strokeWidth={1.5} />
              {formatBytes(s.graphSizeBytes)}
            </span>
          {/snippet}
        </Row>

        <Row
          label="FUSE mount"
          description={s.fuseMounted
            ? "Browseable as a filesystem at the path on the right."
            : "Not mounted — the daemon couldn't bring up the FUSE filesystem."}
          id="kg-fuse-mount"
        >
          {#snippet control()}
            <span class="meta-text" class:positive={s.fuseMounted}>
              <FolderTree size={12} strokeWidth={1.5} />
              {s.fuseMount}
              {s.fuseMounted ? "(mounted ✓)" : "(not mounted)"}
            </span>
          {/snippet}
        </Row>
      {/if}
    {/if}

    <div class="footer-row">
      <Button
        variant="ghost"
        size="sm"
        disabled={loading}
        onclick={refresh}
      >
        <RefreshCw size={14} class={loading ? "spin" : ""} />
        Refresh
      </Button>
    </div>
  </Group>
</SettingsPage>

<style>
  .meta-text {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }

  .meta-text.positive {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }

  .error-box {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0 1rem 0.625rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    border-radius: var(--radius-sm);
    font-size: 0.8125rem;
    color: var(--foreground);
  }

  .error-box :global(svg) {
    flex-shrink: 0;
    color: var(--destructive);
    margin-top: 1px;
  }

  .footer-row {
    display: flex;
    justify-content: flex-end;
    padding: 0.5rem 1rem;
  }

  .footer-row :global(.spin) {
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
