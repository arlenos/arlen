<script lang="ts">
  import { activeMenu, activeAppId, dispatchMenuAction, type MenuItem } from "$lib/stores/menus.js";
  import { activeAppName, activeWindowForOutput } from "$lib/stores/windows.js";
  import { focusedBadge, type BadgeRender } from "$lib/stores/appStateStores";
  import {
    Root, Trigger, Content, Item, Separator, CheckboxItem, Shortcut,
    Sub, SubTrigger, SubContent,
  } from "@arlen/ui-kit/components/ui/dropdown-menu/index.js";
  import { getContext } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import type { Readable } from "svelte/store";
  function handleAction(action: string) {
    const appId = $activeAppId;
    if (appId) dispatchMenuAction(appId, action);
  }

  /// The label of the menu group whose dropdown is open, or null. One
  /// open menu at a time gives standard menubar behaviour: hovering a
  /// sibling trigger switches to it, and an outside click dismisses -
  /// every `Root` is controlled by this single value.
  let openMenu = $state<string | null>(null);

  function onMenuOpenChange(label: string, open: boolean) {
    if (open) openMenu = label;
    else if (openMenu === label) openMenu = null;
  }

  /// Hover-to-switch: once one menu is open, hovering a sibling trigger
  /// switches to it - the menubar convention the applets already use.
  function onTriggerEnter(label: string) {
    if (openMenu !== null) openMenu = label;
  }

  /// A menu whose siblings include a checkbox reserves the leading check
  /// column for ALL its rows, so every label lines up under one edge
  /// (the macOS / GNOME menu convention). Plain items and submenu
  /// triggers then get the same indent as the checked rows.
  function hasChecks(items: MenuItem[]): boolean {
    return items.some((i) => i.type === "item" && i.checked !== undefined);
  }

  /// Each per-output bar mounts its own GlobalMenuBar instance.
  /// We only render the menu when the focused window physically
  /// lives on this monitor — otherwise the user would see the
  /// same menu duplicated on every screen, with no way to tell
  /// which one is the "real" menu for the focused app.
  ///
  /// Pre-resolution (connector === null) the legacy
  /// `activeWindow`-equivalent is returned, so the primary bar's
  /// first paint isn't blank during startup.
  const outputCtx = getContext<
    Readable<{ connector: string | null; primary: boolean }>
  >("topbar-output");
  const outputConnector = $derived($outputCtx?.connector ?? null);
  const windowForThisBar = $derived(activeWindowForOutput(outputConnector));
  let visibleWindowExists = $state(false);
  $effect(() => {
    const unsub = windowForThisBar.subscribe((w) => {
      visibleWindowExists = w !== null;
    });
    return () => unsub();
  });

  // The menu belongs to the focused app; drop any open dropdown when
  // this bar's window is no longer the focused one.
  $effect(() => {
    if (!visibleWindowExists) openMenu = null;
  });

  // The topbar surface only receives pointer input in the 36px bar by
  // default. Expand the layer-shell input region while a menu is open
  // so the dropdown items (drawn below the bar) are clickable and an
  // outside click reaches the webview to dismiss the menu. Mirrors the
  // popovers' `set_popover_input_region` signalling.
  $effect(() => {
    invoke("set_popover_input_region", { expanded: openMenu !== null }).catch(
      () => {},
    );
  });
</script>

{#snippet menuItems(items: MenuItem[])}
  {#each items as item, ii (ii)}
    {#if item.type === "separator"}
      <Separator />
    {:else if item.type === "submenu" && item.children?.length}
      <Sub>
        <SubTrigger>
          {item.label}
        </SubTrigger>
        <SubContent class="menubar-content shell-popover {hasChecks(item.children) ? 'menu-checks' : ''}">
          {@render menuItems(item.children)}
        </SubContent>
      </Sub>
    {:else if item.type === "item" && item.checked !== undefined}
      <CheckboxItem
        checked={item.checked}
        disabled={item.disabled}
        onSelect={() => handleAction(item.action)}
      >
        {item.label}
        {#if item.shortcut}
          <Shortcut>{item.shortcut}</Shortcut>
        {/if}
      </CheckboxItem>
    {:else if item.type === "item"}
      <Item
        disabled={item.disabled}
        onSelect={() => handleAction(item.action)}
      >
        {item.label}
        {#if item.shortcut}
          <Shortcut>{item.shortcut}</Shortcut>
        {/if}
      </Item>
    {/if}
  {/each}
{/snippet}

<div class="menubar">
  {#if visibleWindowExists}
    <span class="menubar-appname">
      {$activeAppName || "Arlen"}
      {#if $focusedBadge}
        {@const b = $focusedBadge as NonNullable<BadgeRender>}
        <span
          class="app-badge"
          class:badge-error={(b.kind === "status" || b.kind === "countWithStatus") && b.status === "error"}
          class:badge-warning={(b.kind === "status" || b.kind === "countWithStatus") && b.status === "warning"}
          class:badge-success={b.kind === "status" && b.status === "success"}
          class:badge-progress={b.kind === "status" && b.status === "progress"}
          class:badge-dot={b.kind === "dot"}
        >
          {#if b.kind === "count"}
            {b.count > 99 ? "99+" : b.count}
          {:else if b.kind === "countWithStatus"}
            {b.count > 99 ? "99+" : b.count}
          {/if}
        </span>
      {/if}
    </span>

    {#if $activeMenu}
    {#each $activeMenu as group, gi (gi)}
      <Root
        open={openMenu === group.label}
        onOpenChange={(o) => onMenuOpenChange(group.label, o)}
      >
        <Trigger>
          {#snippet child({ props })}
            <button
              class="menubar-trigger"
              {...props}
              onmouseenter={() => onTriggerEnter(group.label)}
            >
              {group.label}
            </button>
          {/snippet}
        </Trigger>
        <Content sideOffset={4} class="menubar-content shell-popover {hasChecks(group.items) ? 'menu-checks' : ''}">
          {@render menuItems(group.items)}
        </Content>
      </Root>
    {/each}
    {/if}
  {/if}
</div>

<style>
  .menubar {
    display: flex;
    align-items: center;
    gap: 0;
    height: 100%;
  }

  .menubar-appname {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding: 0 8px;
    position: relative;
  }

  /* App badge: small overlay on the app-name span, same register
     as the topbar unread-count badge. */
  .app-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-left: 4px;
    height: 14px;
    min-width: 14px;
    padding: 0 4px;
    border-radius: var(--radius-full);
    font-size: 9px;
    font-weight: 700;
    line-height: 1;
    color: var(--background);
    background: var(--color-accent);
  }
  .app-badge.badge-dot {
    width: 8px;
    height: 8px;
    min-width: 0;
    padding: 0;
    border-radius: var(--radius-full);
  }
  .app-badge.badge-error {
    background: var(--color-error);
  }
  .app-badge.badge-warning {
    background: var(--color-warning);
  }
  .app-badge.badge-success {
    background: var(--color-success);
  }
  .app-badge.badge-progress {
    background: var(--color-accent);
    animation: badge-progress-pulse 1.4s ease-in-out infinite;
  }
  @keyframes badge-progress-pulse {
    0%, 100% { opacity: 0.4; }
    50% { opacity: 1; }
  }

  .menubar-trigger {
    display: flex;
    align-items: center;
    height: var(--height-control-compact, 24px);
    padding: 0 8px;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-size: 0.75rem;
    font-weight: 500;
    border-radius: var(--radius-chip);
    white-space: nowrap;
    transition: background-color var(--duration-micro, 100ms) ease, color var(--duration-micro, 100ms) ease;
  }

  .menubar-trigger:hover,
  .menubar-trigger[aria-expanded="true"] {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }

  :global(.menubar-content) {
    min-width: 160px;
  }

  /* In a menu that carries a checkbox, the plain items and submenu
     triggers share the leading check column so every label lines up. */
  :global(.menubar-content.menu-checks [data-slot="dropdown-menu-item"]),
  :global(.menubar-content.menu-checks [data-slot="dropdown-menu-sub-trigger"]) {
    padding-left: 2rem;
  }
</style>
