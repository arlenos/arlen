<script lang="ts">
  /// Quick Settings layout customisation page.
  ///
  /// WYSIWYG editor: renders a faithful preview of the desktop-
  /// shell QS panel (380px frame, 2-col tile grid, schematic tiles
  /// with skeleton-replacer for live data) and lets the user drag
  /// to reorder, right-click to resize / hide. Hidden tiles live
  /// in a collapsible drawer below the preview and can be dragged
  /// back in or restored via right-click.
  ///
  /// Visual structure mirrors the rest of the Settings app:
  /// `SettingsPage` shell, `Group` cards from ui-kit (same as
  /// workspaces / appearance / system-actions), banner above the
  /// groups, and a "Reset all" toolbar pinned to the top right
  /// that the user-experiences pattern from system-actions.
  ///
  /// Persistence goes through the generic `config_*` Tauri
  /// commands against `~/.config/lunaris/quicksettings.toml`. The
  /// shell's file-watcher picks up changes automatically.
  ///
  /// Codex review fixes:
  ///   * HIGH-1 (forward-compat): unknown tile ids (module-tier
  ///     `<module>:<tile>` from a future installer) are preserved
  ///     verbatim across save round-trips, never silently dropped.
  ///   * HIGH-2 (parse-fail mutation): if `config_get` fails to
  ///     parse the on-disk file, the editor enters `readOnly`
  ///     mode — drag/resize/hide are all blocked until reload or
  ///     reset succeeds. Without this, the very next click after
  ///     a parse error would overwrite the malformed file with
  ///     stale or default state.
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { RotateCcw, AlertCircle } from "@lucide/svelte";

  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Button } from "$lib/components/ui/button";
  import { ConfirmDialog } from "$lib/components/ui/confirm-dialog";
  import PanelPreview from "$lib/components/quicksettings/PanelPreview.svelte";
  import HiddenDrawer from "$lib/components/quicksettings/HiddenDrawer.svelte";

  type WireSize = "one_by_one" | "two_by_one" | "two_by_two";
  type BodyVariant = "status" | "slider" | "chart" | "footer";

  interface RawTileEntry {
    id: string;
    visible: boolean;
    size: WireSize;
  }
  interface RawLayoutFile {
    tile?: RawTileEntry[];
  }

  /// System-tier tile catalogue. Each entry tells the editor what
  /// to render in the schematic preview (icon, label, allowed
  /// sizes, body shape) without round-tripping to the desktop-
  /// shell. Module-tier (Phase 7) tiles will inject manifest data
  /// into a runtime extension of this map; until then,
  /// catalog-unknown ids round-trip silently in the file but
  /// don't render in the preview.
  const SYSTEM_CATALOG: Record<
    string,
    {
      label: string;
      iconName: string;
      allowed: WireSize[];
      defaultSize: WireSize;
      bodyVariant: BodyVariant;
      /// `true` for tiles whose position is fixed in the real
      /// shell panel (currently `system.user-row` lives at the
      /// bottom and isn't customisable). The editor mirrors the
      /// constraint: pinned tiles render in a separate footer
      /// slot, can't be dragged or hidden, and don't appear in
      /// the hidden-tiles drawer.
      pinned?: boolean;
    }
  > = {
    "system.project-context": {
      label: "Project Context",
      iconName: "folder",
      allowed: ["one_by_one", "two_by_one"],
      defaultSize: "one_by_one",
      bodyVariant: "status",
    },
    "system.knowledge": {
      label: "Knowledge Graph",
      iconName: "brain",
      allowed: ["one_by_one", "two_by_one"],
      defaultSize: "one_by_one",
      bodyVariant: "chart",
    },
    "system.network": {
      label: "Network",
      iconName: "wifi",
      allowed: ["one_by_one"],
      defaultSize: "one_by_one",
      bodyVariant: "status",
    },
    "system.bluetooth": {
      label: "Bluetooth",
      iconName: "bluetooth",
      allowed: ["one_by_one"],
      defaultSize: "one_by_one",
      bodyVariant: "status",
    },
    "system.dnd": {
      label: "Do Not Disturb",
      iconName: "bell-off",
      allowed: ["one_by_one"],
      defaultSize: "one_by_one",
      bodyVariant: "status",
    },
    "system.airplane": {
      label: "Airplane Mode",
      iconName: "plane",
      allowed: ["one_by_one"],
      defaultSize: "one_by_one",
      bodyVariant: "status",
    },
    "system.brightness": {
      label: "Brightness",
      iconName: "sun",
      allowed: ["two_by_one"],
      defaultSize: "two_by_one",
      bodyVariant: "slider",
    },
    "system.audio": {
      label: "Sound",
      iconName: "volume-2",
      allowed: ["two_by_one", "two_by_two"],
      defaultSize: "two_by_one",
      bodyVariant: "slider",
    },
    "system.user-row": {
      label: "User Row",
      iconName: "user",
      allowed: ["two_by_one"],
      defaultSize: "two_by_one",
      bodyVariant: "footer",
      pinned: true,
    },
  };

  const SYSTEM_ORDER: string[] = [
    "system.project-context",
    "system.knowledge",
    "system.network",
    "system.bluetooth",
    "system.dnd",
    "system.airplane",
    "system.brightness",
    "system.audio",
    "system.user-row",
  ];

  let entries = $state<RawTileEntry[]>([]);
  let busy = $state(false);
  let hasLoadedOnce = $state(false);
  let writeSeq = $state(0);
  let confirmResetOpen = $state(false);
  /// `true` after `config_get` failed with a parse/read error.
  /// Blocks every mutation until `load()` succeeds again or the
  /// user clicks "Reset to defaults" (which deletes the
  /// malformed file). Codex HIGH-2.
  let readOnly = $state(false);
  let banner = $state<{ kind: "warning" | "error" | "info"; message: string } | null>(null);

  onMount(load);

  async function load() {
    try {
      const file = await invoke<RawLayoutFile | null>("config_get", {
        file: "quicksettings",
        key: null,
      });
      entries = file?.tile ?? [];
      hasLoadedOnce = true;
      readOnly = false;
      if (banner?.kind === "warning" || banner?.kind === "error") banner = null;
    } catch (err) {
      const isParse = String(err).includes("parse:");
      readOnly = true;
      if (!hasLoadedOnce) {
        entries = [];
      }
      banner = {
        kind: "warning",
        message: isParse
          ? `Quick Settings layout file is malformed. Editor is in read-only mode${
              hasLoadedOnce ? " (showing last saved state)" : ""
            } — fix the file or reset to recover.`
          : `Could not read Quick Settings layout: ${err}`,
      };
    }
  }

  /// Round-trip-safe materialisation. Splits the entries into:
  ///   * `known`: catalogue-known ids the editor can render. The
  ///     editor mutates these via drag/hide/resize.
  ///   * `unknown`: catalogue-foreign ids (module-tier or future
  ///     system tiles this build doesn't recognise yet). They
  ///     round-trip unchanged across saves so a forward-compat
  ///     install never loses module customisation.
  /// Each `persist()` writes `[...known, ...unknown]` so the file
  /// shape stays stable.
  function partition(): { known: RawTileEntry[]; unknown: RawTileEntry[] } {
    const known: RawTileEntry[] = [];
    const unknown: RawTileEntry[] = [];
    const seen = new Set<string>();
    for (const e of entries) {
      const meta = SYSTEM_CATALOG[e.id];
      if (!meta) {
        unknown.push({ ...e });
      } else {
        const size = meta.allowed.includes(e.size) ? e.size : meta.defaultSize;
        known.push({ id: e.id, visible: e.visible, size });
      }
      seen.add(e.id);
    }
    for (const id of SYSTEM_ORDER) {
      if (seen.has(id)) continue;
      const meta = SYSTEM_CATALOG[id];
      known.push({ id, visible: true, size: meta.defaultSize });
    }
    return { known, unknown };
  }

  /// View-models for the preview + drawer. Re-derived from
  /// `entries` so any state mutation reflows the UI without
  /// round-tripping through disk.
  const visibleTiles = $derived.by(() => {
    return partition()
      .known.filter((e) => e.visible)
      .map((e) => {
        const meta = SYSTEM_CATALOG[e.id]!;
        return {
          id: e.id,
          label: meta.label,
          iconName: meta.iconName,
          size: e.size,
          allowedSizes: meta.allowed,
          bodyVariant: meta.bodyVariant,
          pinned: meta.pinned ?? false,
        };
      });
  });

  /// Pinned tiles can't be hidden via the editor, so they never
  /// appear in this list — preserves the constraint that tiles
  /// like the user-row are always present.
  const hiddenTiles = $derived.by(() => {
    return partition()
      .known.filter((e) => !e.visible && !SYSTEM_CATALOG[e.id]?.pinned)
      .map((e) => {
        const meta = SYSTEM_CATALOG[e.id]!;
        return { id: e.id, label: meta.label, iconName: meta.iconName };
      });
  });

  /// Single-flight versioned save. Carries `unknown` entries
  /// untouched (Codex HIGH-1) and rolls back on error only when
  /// no newer write has been started since.
  async function persist(nextKnown: RawTileEntry[]) {
    if (readOnly) return;
    const myVersion = ++writeSeq;
    const previous = entries;
    busy = true;
    const { unknown } = partition();
    const next = [...nextKnown, ...unknown];
    entries = next;
    try {
      await invoke("config_set", {
        file: "quicksettings",
        key: "tile",
        value: next,
      });
    } catch (e) {
      if (myVersion === writeSeq) {
        entries = previous;
        banner = { kind: "error", message: `Save failed: ${e}` };
      }
    }
    if (myVersion === writeSeq) busy = false;
  }

  /// Reorder a tile within the visible-draggable list. The
  /// `toIndex` is in the same coordinate space as the rendered
  /// preview grid (visible non-pinned tiles only) — translating
  /// it to a position inside `partition().known` (which mixes
  /// visible + hidden + pinned) would land entries at the wrong
  /// slot whenever a hidden tile happened to sit between
  /// visible ones. Working in the visible-only space and
  /// re-merging hidden + pinned at write time keeps the math
  /// simple and the saved file canonical:
  /// `[...visible-non-pinned, ...hidden, ...pinned]`.
  function reorder(fromId: string, toIndex: number) {
    if (readOnly || busy) return;
    if (SYSTEM_CATALOG[fromId]?.pinned) return;
    const all = partition().known;
    const visibleDraggable = all.filter(
      (e) => e.visible && !SYSTEM_CATALOG[e.id]?.pinned,
    );
    const rest = all.filter(
      (e) => !e.visible || SYSTEM_CATALOG[e.id]?.pinned,
    );

    let moved: typeof all[number] | undefined;
    const fromVis = visibleDraggable.findIndex((e) => e.id === fromId);
    if (fromVis >= 0) {
      [moved] = visibleDraggable.splice(fromVis, 1);
    } else {
      // Source is in `rest` (hidden) — un-hide and drop into
      // the visible list at the requested position.
      const fromHidden = rest.findIndex((e) => e.id === fromId);
      if (fromHidden < 0) return;
      [moved] = rest.splice(fromHidden, 1);
      moved.visible = true;
    }
    if (!moved) return;

    const target = Math.min(toIndex, visibleDraggable.length);
    visibleDraggable.splice(target, 0, moved);
    persist([...visibleDraggable, ...rest]);
  }

  function setSize(id: string, size: WireSize) {
    if (readOnly || busy) return;
    const list = partition().known;
    const idx = list.findIndex((e) => e.id === id);
    if (idx < 0) return;
    list[idx].size = size;
    persist(list);
  }

  function hideTile(id: string) {
    if (readOnly || busy) return;
    if (SYSTEM_CATALOG[id]?.pinned) return;
    const list = partition().known;
    const idx = list.findIndex((e) => e.id === id);
    if (idx < 0) return;
    list[idx].visible = false;
    persist(list);
  }

  function showTile(id: string) {
    if (readOnly || busy) return;
    const list = partition().known;
    const idx = list.findIndex((e) => e.id === id);
    if (idx < 0) return;
    const [moved] = list.splice(idx, 1);
    moved.visible = true;
    const lastVisible = list.findLastIndex((e) => e.visible);
    list.splice(lastVisible + 1, 0, moved);
    persist(list);
  }

  async function resetAll() {
    busy = true;
    try {
      await invoke("config_reset", { file: "quicksettings", key: null });
      await load();
      banner = {
        kind: "info",
        message: "Quick Settings layout restored to defaults.",
      };
    } catch (e) {
      banner = { kind: "error", message: `Reset failed: ${e}` };
    }
    busy = false;
    confirmResetOpen = false;
  }

  const { unknown: unknownEntries } = $derived(partition());
</script>

<SettingsPage
  title="Quick Settings"
  description="Reorder, hide, and resize the tiles inside the Quick Settings panel. Drag to rearrange. Right-click a tile for size and visibility controls."
>
  <div class="qs-toolbar">
    <Button
      variant="ghost"
      size="sm"
      onclick={() => (confirmResetOpen = true)}
      disabled={busy}
    >
      <RotateCcw size={14} strokeWidth={1.5} />
      Reset to defaults
    </Button>
  </div>

  {#if banner}
    <div class="qs-banner kind-{banner.kind}">
      <AlertCircle size={16} strokeWidth={1.75} />
      <span>{banner.message}</span>
    </div>
  {/if}

  <Group label="Layout">
    <div class="qs-layout-row" id="qs-layout-list">
      <div class="qs-layout-stage">
        <PanelPreview
          tiles={visibleTiles}
          {readOnly}
          onReorder={reorder}
          onResize={setSize}
          onHide={hideTile}
        />
        <HiddenDrawer tiles={hiddenTiles} onShow={showTile} />
      </div>
    </div>
  </Group>

  {#if unknownEntries.length > 0}
    <Group label="Module tiles">
      <div class="qs-unknown-row">
        <p>
          {unknownEntries.length} tile{unknownEntries.length === 1 ? "" : "s"}
          installed by other modules are part of your layout but not editable
          here. They are preserved on save.
        </p>
        <ul>
          {#each unknownEntries as e (e.id)}
            <li><code>{e.id}</code></li>
          {/each}
        </ul>
      </div>
    </Group>
  {/if}
</SettingsPage>

<ConfirmDialog
  open={confirmResetOpen}
  title="Reset Quick Settings layout?"
  message="This restores the bundled tile order, sizes, and visibility. Your customisation will be lost."
  confirmLabel="Reset"
  variant="destructive"
  onConfirm={resetAll}
  onCancel={() => (confirmResetOpen = false)}
/>

<style>
  /* Toolbar pinned to the top-right above the groups — same
     pattern as system-actions's "Reset all to defaults". */
  .qs-toolbar {
    display: flex;
    justify-content: flex-end;
    margin-top: -0.5rem;
    margin-bottom: -0.25rem;
  }

  /* Layout-group content wrapper. The Group card spans the full
     content width; we centre the 380px PanelPreview within it
     so the editor reads as the actual panel sitting in the
     middle of the page rather than glued to the left edge. */
  .qs-layout-row {
    padding: 1rem;
  }
  .qs-layout-stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
  }

  .qs-banner {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--radius-input);
    font-size: 0.8125rem;
    line-height: 1.4;
  }
  .qs-banner.kind-warning {
    background: color-mix(in srgb, var(--color-warning, #eab308) 14%, transparent);
    color: color-mix(in srgb, var(--color-warning, #eab308) 90%, var(--color-fg-shell));
    border: 1px solid color-mix(in srgb, var(--color-warning, #eab308) 30%, transparent);
  }
  .qs-banner.kind-error {
    background: color-mix(in srgb, var(--color-error, #ef4444) 14%, transparent);
    color: color-mix(in srgb, var(--color-error, #ef4444) 90%, var(--color-fg-shell));
    border: 1px solid color-mix(in srgb, var(--color-error, #ef4444) 30%, transparent);
  }
  .qs-banner.kind-info {
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 80%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
  }

  .qs-unknown-row {
    padding: 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-shell) 75%, transparent);
  }
  .qs-unknown-row p {
    margin: 0 0 0.5rem 0;
  }
  .qs-unknown-row ul {
    margin: 0;
    padding-left: 1rem;
  }
  .qs-unknown-row code {
    font-size: 0.75rem;
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    padding: 1px 6px;
    border-radius: var(--radius-chip);
  }
</style>
