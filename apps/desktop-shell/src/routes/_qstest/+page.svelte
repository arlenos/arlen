<script lang="ts">
  /// Headless look-mock for the Quick Settings panel, so its copy can be read by
  /// the screenshot loop. The panel lives behind a top-bar trigger that only
  /// exists once Tauri is present, so under plain vite it cannot be reached at
  /// all - and its empty state carries a whole sentence with a button inside it,
  /// which is exactly the shape that goes wrong quietly.
  ///
  /// `?locale=de` renders it in German. Not in any nav; the `_undotest` pattern.
  import { onMount } from "svelte";
  import QuickSettingsPanel from "$lib/components/QuickSettingsPanel.svelte";
  import { openPopover } from "$lib/stores/activePopover.js";
  import { locale } from "@arlen/ui-kit/i18n";

  // Runs at module init, before the panel mounts and reads its layout. `onMount`
  // would be too late: a parent's runs after its children's.
  if (typeof window !== "undefined") {
    const params = new URLSearchParams(window.location.search);
    if (params.get("locale")) locale.set(params.get("locale") as string);
    if (params.get("state") === "empty") {
      // Answer the layout read with nothing, so the empty state is reachable.
      // Under vite there is no Tauri at all, and the panel's own fallback fills
      // in a default set of tiles - so without this the one piece of copy worth
      // looking at here can never be seen.
      // Every bundled tile hidden. An empty list is not enough: `resolveLayout`
      // appends any system tile the user file does not mention, so "no entries"
      // means "all defaults", which is the opposite of what this state is.
      const hidden = [
        "system.user-row",
        "system.project-context",
        "system.knowledge",
        "system.network",
        "system.dnd",
        "system.airplane",
        "system.audio",
        "system.bluetooth",
        "system.brightness",
      ].map((id) => ({ id, visible: false, size: "1x1" }));
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: async (cmd: string) => (cmd === "qs_layout_get" ? { tile: hidden } : undefined),
      };
    }
  }

  onMount(() => {
    openPopover("quick-settings");
  });
</script>

<div class="qs-stage">
  <QuickSettingsPanel />
</div>

<style>
  .qs-stage {
    min-height: 100vh;
    display: flex;
    justify-content: flex-end;
    padding: 44px 12px 0 0;
    background: var(--color-bg-shell, #0a0a0a);
  }
</style>
