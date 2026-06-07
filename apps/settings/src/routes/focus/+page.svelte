<script lang="ts">
  /// Focus Mode settings page.
  ///
  /// Focus-specific settings only: the top-bar indicator and the default
  /// suppressed-apps list, both backed by shell.toml [focus_settings].
  ///
  /// Project detection (watch directories, recursion depth, auto-promote
  /// threshold → graph.toml [projects]) is a Knowledge-Graph concern and is
  /// edited on the Knowledge page. This page links there rather than offering a
  /// second editor for the same keys.

  import { onMount } from "svelte";
  import { AppWindow, FolderSearch } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { AddRemoveList } from "$lib/components/ui/add-remove-list";
  import { shell, FOCUS_SETTINGS_DEFAULTS } from "$lib/stores/shell";
  import AppPicker from "$lib/components/appearance/AppPicker.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { navigateTo } from "$lib/stores/navigation";

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

  async function setShowProjectName(v: boolean) {
    await shell.setValue("focus_settings.show_project_name", v);
  }

  async function addSuppressedApp(name: string) {
    if (!name.trim()) return;
    if (suppressedApps.includes(name)) return;
    await shell.setValue("focus_settings.default_suppressed_apps", [
      ...suppressedApps,
      name,
    ]);
  }

  async function removeSuppressedApp(index: number) {
    await shell.setValue(
      "focus_settings.default_suppressed_apps",
      suppressedApps.filter((_, i) => i !== index),
    );
  }
</script>

<Page
  title="Focus Mode"
  description="The top-bar indicator and which apps are silenced while Focus Mode is active."
>
  <SectionGrid>
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
        {#snippet itemSnippet({ item }: { item: string; index: number })}
          <span class="app-row">
            <AppWindow size={14} strokeWidth={1.5} />
            {item}
          </span>
        {/snippet}
      </AddRemoveList>
    </div>
  </Group>

  <Group label="Project Detection">
    <Row
      label="Configured on the Knowledge page"
      description="Which directories are scanned for projects, the recursion depth, and the auto-promote threshold are part of the knowledge graph. Focus Mode uses the projects it detects."
      id="focus-project-detection-link"
    >
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() => navigateTo("knowledge", "kg-watch-dirs")}
        >
          <FolderSearch size={14} />
          Open Knowledge
        </Button>
      {/snippet}
    </Row>
    </Group>
  </SectionGrid>
</Page>

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

  .app-row {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
</style>
