<script lang="ts">
  /// Topbar settings: arrange the applets + tray items in the desktop-shell top
  /// bar (drag to reorder) and toggle each shown-in-bar vs in-overflow. A live
  /// preview reflects the shown set + order. First cut - reorder + visibility;
  /// zone-switching is later polish.
  import { onMount } from "svelte";
  import type { Component } from "svelte";
  import {
    Bell,
    Volume2,
    Wifi,
    Bluetooth,
    BatteryFull,
    LayoutGrid,
    Clock,
    Square,
    AppWindow,
    GripVertical,
    MoreHorizontal,
  } from "@lucide/svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { t } from "$lib/i18n/messages";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { SortableList } from "@arlen/ui-kit/components/ui/sortable-list";
  import { topbar, shownItems, load, reorder, setShown, type TopbarItem } from "$lib/stores/topbar";

  onMount(() => void load());

  type Glyph = Component<{ size?: number | string; strokeWidth?: number | string }>;
  const ICONS: Record<string, Glyph> = {
    notifications: Bell,
    audio: Volume2,
    network: Wifi,
    bluetooth: Bluetooth,
    battery: BatteryFull,
    layout: LayoutGrid,
    clock: Clock,
    "quick-settings": Square,
  };
  const iconFor = (item: TopbarItem): Glyph => ICONS[item.icon] ?? AppWindow;

  const order = $derived($topbar.items.map((i) => i.id));
  const byId = (id: string): TopbarItem | undefined => $topbar.items.find((i) => i.id === id);
  const hasOverflow = $derived($topbar.items.some((i) => !i.shown));
</script>

<Page title={$t("s.topbar.title")} description={$t("s.topbar.desc")}>
  <SectionGrid>
    <Group label={$t("s.topbar.preview")} class="span-full">
      <div class="tb-preview" aria-label={$t("s.topbar.previewAria")}>
        <span class="tb-pv-left">Arlen</span>
        <span class="tb-pv-spacer"></span>
        {#each $shownItems as item (item.id)}
          {@const Icon = iconFor(item)}
          <span class="tb-pv-icon"><Icon size={15} strokeWidth={1.75} /></span>
        {/each}
        {#if hasOverflow}
          <span class="tb-pv-icon tb-pv-overflow"><MoreHorizontal size={15} strokeWidth={1.75} /></span>
        {/if}
      </div>
    </Group>

    <Group label={$t("s.topbar.applets")} class="span-full">
      {#if $topbar.items.length > 0}
        <SortableList ids={order} onReorder={reorder}>
          {#snippet item(id)}
            {@const it = byId(id)}
            {#if it}
              {@const Icon = iconFor(it)}
              <div class="tb-row" class:dimmed={!it.shown}>
                <button class="tb-handle" data-sortable-handle aria-label={`Reorder ${it.name}`}>
                  <GripVertical size={15} strokeWidth={2} />
                </button>
                <span class="tb-icon"><Icon size={16} strokeWidth={1.75} /></span>
                <span class="tb-name">{it.name}</span>
                {#if it.kind === "tray"}<span class="tb-tag">tray</span>{/if}
                <span class="tb-spacer"></span>
                <span class="tb-state">{it.shown ? "Shown" : "Overflow"}</span>
                <Switch value={it.shown} onchange={(v) => setShown(it.id, v)} />
              </div>
            {/if}
          {/snippet}
        </SortableList>
      {/if}
    </Group>

    {#if $topbar.error}
      <div class="span-full tb-error" title={$topbar.error}>
        Can't read the topbar arrangement right now. Changes are paused.
      </div>
    {/if}
  </SectionGrid>
</Page>

<style>
  .tb-preview {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 12px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .tb-pv-left {
    font-size: var(--text-sm);
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .tb-pv-spacer {
    flex: 1;
  }
  .tb-pv-icon {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .tb-pv-overflow {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  .tb-row {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 2.5rem;
    padding: 0 4px 0 2px;
  }
  /* A hairline between rows; the first row's top divider reads against the
     group label cleanly. */
  :global(.sortable-row + .sortable-row) .tb-row {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .tb-handle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.75rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    cursor: grab;
    touch-action: none;
  }
  .tb-handle:hover {
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .tb-handle:active {
    cursor: grabbing;
  }
  .tb-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .tb-name {
    font-size: var(--text-sm);
    color: var(--foreground);
  }
  .tb-row.dimmed .tb-name,
  .tb-row.dimmed .tb-icon {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .tb-tag {
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .tb-spacer {
    flex: 1;
  }
  .tb-state {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .tb-error {
    padding: 0.75rem;
    border-radius: var(--radius-chip, 4px);
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    font-size: var(--text-sm);
    color: var(--destructive);
  }
</style>
