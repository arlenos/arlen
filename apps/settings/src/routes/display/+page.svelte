<script lang="ts">
  /// Display panel.
  ///
  /// Composes MonitorMap (visual layout) + MonitorSidePanel (per-
  /// monitor controls) + RevertConfirmModal (15s revert safety).
  /// All state lives in `$lib/stores/displays`; the page wires
  /// them together and calls `applyConfig` when the user hits
  /// Apply.

  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import {
    monitors,
    selectedConnector,
    initDisplayStore,
    applyConfig,
    monitorToConfig,
    type Monitor,
    type MonitorConfig,
    type Position,
  } from "$lib/stores/displays";
  import MonitorMap from "$lib/components/displays/MonitorMap.svelte";
  import MonitorSidePanel from "$lib/components/displays/MonitorSidePanel.svelte";
  import NightLightSection from "$lib/components/displays/NightLightSection.svelte";
  import BrightnessSection from "$lib/components/displays/BrightnessSection.svelte";
  import RevertConfirmModal from "$lib/components/displays/RevertConfirmModal.svelte";
  import { Button } from "$lib/components/ui/button";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsGroup from "$lib/components/settings/SettingsGroup.svelte";

  // We deliberately do NOT mirror $monitors / $selectedConnector
  // into local `$state`. Svelte 5's runes scheduler does not detect
  // mutations driven by Tauri-IPC `writable.set` callbacks, so any
  // `$state` populated via `store.subscribe(...)` ends up stale
  // (CLAUDE.md). Template-level `$store` auto-subscribe is the only
  // reliable read path; everything reactive happens inline in the
  // template below.

  // Drafts: the per-connector config the user is editing. Seeding
  // is implicit — when a connector has no draft, the side panel
  // falls back to `monitorToConfig(monitor)`. This avoids needing a
  // `$effect` to keep drafts in sync (which would also be defeated
  // by the same scheduler limitation).
  let drafts = $state<Record<string, MonitorConfig>>({});

  // Revert-modal state.
  let modalOpen = $state(false);
  let modalSnapshot = $state<MonitorConfig[]>([]);
  let modalRequestId = $state<string | null>(null);
  let applyError = $state<string | null>(null);

  let teardown: (() => void) | null = null;

  onMount(async () => {
    teardown = await initDisplayStore();
  });

  onDestroy(() => {
    teardown?.();
  });

  function updateDraft(connector: string, draft: MonitorConfig) {
    drafts = { ...drafts, [connector]: draft };
  }

  function updatePosition(connector: string, pos: Position) {
    const cur = drafts[connector];
    if (cur) {
      updateDraft(connector, { ...cur, position: pos });
    } else {
      // No draft yet — seed from the live monitor and apply the
      // pointer-driven position.
      const m = get(monitors).find((m) => m.connector === connector);
      if (m) updateDraft(connector, { ...monitorToConfig(m), position: pos });
    }
  }

  function isDirty(
    mons: Monitor[],
    drs: Record<string, MonitorConfig>,
  ): boolean {
    for (const m of mons) {
      const d = drs[m.connector];
      if (!d) continue;
      const live = monitorToConfig(m);
      if (JSON.stringify(d) !== JSON.stringify(live)) return true;
    }
    return false;
  }

  async function onApply() {
    applyError = null;
    // Read the live monitor list imperatively so we capture exactly
    // what's on screen at click time.
    const live = get(monitors);
    const config = live
      .map((m) => drafts[m.connector] ?? monitorToConfig(m))
      .filter((c): c is MonitorConfig => c !== undefined);
    try {
      const handle = await applyConfig(config);
      modalSnapshot = handle.snapshot;
      modalRequestId = handle.requestId;
      modalOpen = true;
    } catch (err) {
      applyError = String(err);
    }
  }

  function onModalClose() {
    modalOpen = false;
    modalRequestId = null;
  }

  function selectMonitor(connector: string) {
    selectedConnector.set(connector);
  }
</script>

<SettingsPage
  title="Displays"
  description="Drag the boxes to rearrange. Pick a display to tune resolution, scale, rotation, mirror mode, and more."
>
  <SettingsGroup label="Arrangement">
    <div class="map-wrap">
      <MonitorMap
        {drafts}
        selected={$selectedConnector}
        onSelect={selectMonitor}
        onPositionChange={updatePosition}
      />
    </div>
  </SettingsGroup>

  {#if $monitors.length > 0}
    {@const selectedMonitor =
      $monitors.find((m) => m.connector === $selectedConnector) ?? null}
    {@const otherMonitors =
      $monitors.filter((m) => m.connector !== $selectedConnector)}
    {@const dirty = isDirty($monitors, drafts)}

    {#if selectedMonitor}
      <SettingsGroup label={selectedMonitor.connector}>
        <MonitorSidePanel
          monitor={selectedMonitor}
          draft={drafts[selectedMonitor.connector] ?? monitorToConfig(selectedMonitor)}
          others={otherMonitors}
          onChange={(d) => updateDraft(selectedMonitor.connector, d)}
        />
        <div class="action-row">
          {#if applyError}
            <p class="apply-error">{applyError}</p>
          {/if}
          <Button variant="outline" onclick={onApply} disabled={!dirty}>
            Apply
          </Button>
        </div>
      </SettingsGroup>
    {/if}
  {/if}

  <BrightnessSection />
  <NightLightSection />
</SettingsPage>

<RevertConfirmModal
  open={modalOpen}
  snapshot={modalSnapshot}
  requestId={modalRequestId}
  onClose={onModalClose}
/>

<style>
  /* MonitorMap is the canvas — let it use the SettingsGroup card's
     full width without inner padding so the rectangle preview can
     fill the bounds. */
  .map-wrap {
    padding: 12px;
  }

  /* Apply lives as a footer row inside the side-panel card so the
     `divide-y` of `SettingsGroup` draws a separator above it; same
     padding as `SettingsRow` (`px-4 py-3`). Right-aligned so the
     button trails the row and the user's eye lands on it after
     scanning the controls. */
  .action-row {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    padding: 12px 16px;
  }

  .apply-error {
    margin: 0;
    color: var(--destructive);
    font-size: 0.85rem;
  }
</style>
