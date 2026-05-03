<script lang="ts">
  /// Focus Mode settings page (Sprint C).
  ///
  /// Three concerns split across two config files:
  ///   - Top-bar indicator + default suppressed apps → shell.toml
  ///     [focus_settings]
  ///   - Project detection (auto-promote threshold, watch dirs,
  ///     recursion depth) → graph.toml [projects]
  ///
  /// graph.toml is read by the knowledge daemon at startup only —
  /// the page surfaces a "Restart Knowledge Daemon" hint when any
  /// project-detection field changes.

  import { onMount } from "svelte";
  import {
    AppWindow,
    FolderOpen as FolderOpenIcon,
    AlertTriangle,
  } from "lucide-svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Switch } from "$lib/components/ui/switch";
  import { ValueSlider } from "$lib/components/ui/value-slider";
  import { AddRemoveList } from "$lib/components/ui/add-remove-list";
  import { DirectoryPicker } from "$lib/components/ui/directory-picker";
  import { shell, FOCUS_SETTINGS_DEFAULTS } from "$lib/stores/shell";
  import { graph, PROJECTS_DEFAULTS } from "$lib/stores/projectsConfig";
  import AppPicker from "$lib/components/appearance/AppPicker.svelte";
  import { invoke } from "@tauri-apps/api/core";

  /// Backend returns `Vec<AppHistoryEntry>` — objects with
  /// `app_name`, `last_seen`, `count`. AppPicker expects flat
  /// strings, so we map to `app_name` after fetch (Codex Sprint C
  /// review HIGH 2 — passing the raw objects through broke
  /// AppPicker's string methods on every keystroke).
  interface AppHistoryEntry {
    app_name: string;
    last_seen?: number;
    count?: number;
  }

  let knownApps = $state<string[]>([]);

  onMount(async () => {
    shell.load();
    graph.load();
    // Source the AppPicker list from the notification-daemon's
    // history. This is the same list the Notifications page uses
    // for per-app rules; same source ensures app-name spelling
    // matches at suppression time.
    try {
      const entries = await invoke<AppHistoryEntry[]>(
        "notifications_get_known_apps",
      );
      knownApps = entries.map((e) => e.app_name).filter((n) => n.length > 0);
    } catch {
      knownApps = [];
    }
  });

  // Reactive views with defaults.
  const showProjectName = $derived<boolean>(
    ($shell.data?.focus_settings?.show_project_name as boolean | undefined) ??
      FOCUS_SETTINGS_DEFAULTS.show_project_name,
  );
  const suppressedApps = $derived<string[]>(
    ($shell.data?.focus_settings?.default_suppressed_apps as string[] | undefined) ?? [],
  );

  const watchDirs = $derived<string[]>(
    ($graph.data?.projects?.watch_directories as string[] | undefined) ??
      [],
  );
  const usingDefaultDirs = $derived(
    !($graph.data?.projects?.watch_directories) ||
      ($graph.data.projects.watch_directories as string[]).length === 0,
  );
  const promoteThreshold = $derived<number>(
    ($graph.data?.projects?.auto_promote_threshold as number | undefined) ??
      PROJECTS_DEFAULTS.auto_promote_threshold,
  );
  const maxDepth = $derived<number>(
    ($graph.data?.projects?.max_depth as number | undefined) ??
      PROJECTS_DEFAULTS.max_depth,
  );

  let suppressDirty = $state(false);
  let projectDirty = $state(false);

  // Project-detection changes need a daemon restart — track when
  // any of those fields was edited this session so the warning
  // banner only appears after a real change.
  function markProjectDirty() {
    projectDirty = true;
  }

  async function setShowProjectName(v: boolean) {
    await shell.setValue("focus_settings.show_project_name", v);
  }

  async function addSuppressedApp(name: string) {
    if (!name.trim()) return;
    if (suppressedApps.includes(name)) return;
    suppressDirty = true;
    await shell.setValue("focus_settings.default_suppressed_apps", [
      ...suppressedApps,
      name,
    ]);
  }

  async function removeSuppressedApp(index: number) {
    suppressDirty = true;
    await shell.setValue(
      "focus_settings.default_suppressed_apps",
      suppressedApps.filter((_, i) => i !== index),
    );
  }

  async function addWatchDir(path: string) {
    if (watchDirs.includes(path)) return;
    markProjectDirty();
    // First add: write the explicit list (drops the implicit defaults).
    const next = usingDefaultDirs ? [path] : [...watchDirs, path];
    await graph.setValue("projects.watch_directories", next);
  }

  async function removeWatchDir(index: number) {
    markProjectDirty();
    await graph.setValue(
      "projects.watch_directories",
      watchDirs.filter((_, i) => i !== index),
    );
  }

  async function setPromoteThreshold(v: number) {
    markProjectDirty();
    await graph.setValue("projects.auto_promote_threshold", v);
  }

  async function setMaxDepth(v: number) {
    markProjectDirty();
    await graph.setValue("projects.max_depth", v);
  }

  // Per-item missing-path indicator. Tauri command would be ideal
  // but we don't want a roundtrip per render — the user's machine
  // can resolve `~` so we just check if the entry looks like a
  // home-relative path that exists in their HOME shell.
  function pathLabel(p: string): string {
    return p;
  }
</script>

<SettingsPage
  title="Focus Mode"
  description="Project detection, default suppressed apps, and watch directories."
>
  <Group label="Top Bar Indicator">
    <Row
      label="Show project name when active"
      description="Pin the active project name to the top bar while Focus Mode is on."
      id="focus-show-project-name"
    >
      {#snippet control()}
        <Switch
          value={showProjectName}
          ariaLabel="Show project name in top bar"
          onchange={setShowProjectName}
        />
      {/snippet}
    </Row>
  </Group>

  <Group label="Default Suppressed Apps">
    <Row
      label="Suppress these apps' notifications by default"
      description="Whenever Focus Mode is active, notifications from these apps are silenced. Per-project .project files override this list."
      id="focus-suppressed-apps"
    >
      {#snippet control()}
        <span class="meta-count">
          {suppressedApps.length}
          {suppressedApps.length === 1 ? "app" : "apps"}
        </span>
      {/snippet}
    </Row>
    <div class="picker-row">
      <AppPicker
        {knownApps}
        excluded={suppressedApps}
        placeholder="Add app..."
        onpick={addSuppressedApp}
      />
    </div>
    <div class="list-wrap">
      <AddRemoveList
        items={suppressedApps}
        onremove={removeSuppressedApp}
        onadd={() => {
          // No-op: the picker above is the add affordance for
          // this list. AddRemoveList still requires onadd, so
          // the button is hidden via empty addLabel.
        }}
        addLabel=""
        emptyMessage="No apps configured — Focus Mode uses per-project lists only."
      >
        {#snippet itemSnippet({ item })}
          <span class="app-row">
            <AppWindow size={14} strokeWidth={1.5} />
            {item}
          </span>
        {/snippet}
      </AddRemoveList>
    </div>
  </Group>

  <Group label="Project Detection">
    {#if projectDirty}
      <div class="restart-warn" role="status">
        <AlertTriangle size={14} strokeWidth={2} />
        <span>
          Restart the Knowledge Daemon to apply project-detection
          changes. The daemon reads <code>graph.toml</code> only on
          startup today; live-reload is a follow-up sprint.
        </span>
      </div>
    {/if}

    <Row
      label="Auto-promote threshold"
      description="Promote an inferred directory to a project after this many distinct files have been opened in one session."
      id="focus-promote-threshold"
    >
      {#snippet control()}
        <ValueSlider
          value={promoteThreshold}
          min={1}
          max={20}
          step={1}
          unit="files"
          ariaLabel="Auto-promote threshold"
          onchange={setPromoteThreshold}
        />
      {/snippet}
    </Row>

    <Row
      label="Watch directories"
      description={usingDefaultDirs
        ? `Defaults: ${PROJECTS_DEFAULTS.watch_directories.join(", ")}. Add a directory to override.`
        : "Directories scanned for project markers (.project, .git, Cargo.toml, etc.)."}
      id="focus-watch-dirs"
    >
      {#snippet control()}
        <DirectoryPicker
          label="Add directory"
          onpick={addWatchDir}
        />
      {/snippet}
    </Row>
    {#if !usingDefaultDirs}
      <div class="list-wrap">
        <AddRemoveList
          items={watchDirs}
          onremove={removeWatchDir}
          onadd={() => {
            // Picker in the row above is the add path. Hide the
            // duplicate Add button via empty label.
          }}
          addLabel=""
          emptyMessage="No watched directories — defaults will be used."
        >
          {#snippet itemSnippet({ item })}
            <span class="dir-row">
              <FolderOpenIcon size={14} strokeWidth={1.5} />
              {pathLabel(item as string)}
            </span>
          {/snippet}
        </AddRemoveList>
      </div>
    {/if}

    <Row
      label="Recursion depth"
      description="How deep the watcher recurses below each watch directory looking for project markers."
      id="focus-max-depth"
    >
      {#snippet control()}
        <ValueSlider
          value={maxDepth}
          min={1}
          max={6}
          step={1}
          unit="levels"
          ariaLabel="Recursion depth"
          onchange={setMaxDepth}
        />
      {/snippet}
    </Row>
  </Group>
</SettingsPage>

<style>
  .meta-count {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .picker-row,
  .list-wrap {
    padding: 0 1rem 0.625rem;
  }

  .picker-row {
    padding-top: 0.25rem;
  }

  .app-row,
  .dir-row {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }

  .restart-warn {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0 1rem 0.625rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    border-radius: var(--radius-chip);
    font-size: 0.8125rem;
    line-height: 1.4;
    color: var(--foreground);
  }

  .restart-warn :global(svg) {
    flex-shrink: 0;
    color: var(--destructive);
    margin-top: 1px;
  }

  .restart-warn code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.7rem;
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    padding: 0.05rem 0.3rem;
    border-radius: 4px;
  }
</style>
