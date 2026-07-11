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
  import { AddRemoveList } from "@arlen/ui-kit/components/ui/add-remove-list";
  import { shell, FOCUS_SETTINGS_DEFAULTS } from "$lib/stores/shell";
  import { t } from "$lib/i18n/messages";
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
  title={$t("s.focus.title")}
  description={$t("s.focus.desc")}
>
  <SectionGrid>
    <Group label={$t("s.focus.topBar")}>
    <Row
      label={$t("s.focus.showProject")}
      description={$t("s.focus.showProject.desc")}
      id="focus-show-project-name"
    >
      {#snippet control()}
        <Switch
          value={showProjectName}
          ariaLabel={$t("s.focus.showProject.aria")}
          onchange={setShowProjectName}
        />
      {/snippet}
    </Row>
  </Group>

  <Group label={$t("s.focus.suppressed")}>
    <Row
      label={$t("s.focus.suppress")}
      description={$t("s.focus.suppress.desc")}
      id="focus-suppressed-apps"
    >
      {#snippet control()}
        <span class="meta-count">
          {$t("s.focus.appCount", { count: suppressedApps.length })}
        </span>
      {/snippet}
    </Row>
    <div class="picker-list">
      <AppPicker
        {knownApps}
        excluded={suppressedApps}
        placeholder={$t("s.focus.addApp")}
        onpick={addSuppressedApp}
      />
      <AddRemoveList
        items={suppressedApps}
        onremove={removeSuppressedApp}
        onadd={() => {
          // No-op: the picker above is the add affordance for
          // this list. AddRemoveList still requires onadd, so
          // the button is hidden via empty addLabel.
        }}
        addLabel=""
        emptyMessage={$t("s.focus.noApps")}
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

  <Group label={$t("s.focus.projectDetection")}>
    <Row
      label={$t("s.focus.configKnowledge")}
      description={$t("s.focus.configKnowledge.desc")}
      id="focus-project-detection-link"
    >
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() => navigateTo("knowledge", "kg-watch-dirs")}
        >
          <FolderSearch size={14} />
          {$t("s.focus.openKnowledge")}
        </Button>
      {/snippet}
    </Row>
    </Group>
  </SectionGrid>
</Page>

<style>
  .meta-count {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* One card child for picker + list: one divider above the pair, one
     internal rhythm, so the empty box never sits flush on a border. */
  .picker-list {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    padding: 0.625rem 1rem;
  }


  .app-row {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
</style>
