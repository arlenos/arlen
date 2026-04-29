<script lang="ts">
  import { activeMenu, activeAppId, dispatchMenuAction, type MenuGroup, type MenuItem } from "$lib/stores/menus.js";
  import { activeAppName, activeWindowForOutput } from "$lib/stores/windows.js";
  import {
    Root, Trigger, Content, Item, Separator, CheckboxItem, Shortcut,
    Sub, SubTrigger, SubContent,
  } from "$lib/components/ui/dropdown-menu/index.js";
  import { getContext } from "svelte";
  import type { Readable } from "svelte/store";
  const shellColors =
    "bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  function handleAction(action: string) {
    const appId = $activeAppId;
    if (appId) dispatchMenuAction(appId, action);
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
        <SubContent class="menubar-content {shellColors}">
          {@render menuItems(item.children)}
        </SubContent>
      </Sub>
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
      {$activeAppName || "Lunaris"}
    </span>

    {#if $activeMenu}
    {#each $activeMenu as group, gi (gi)}
      <Root>
        <Trigger>
          {#snippet child({ props })}
            <button class="menubar-trigger" {...props}>
              {group.label}
            </button>
          {/snippet}
        </Trigger>
        <Content sideOffset={4} class="menubar-content {shellColors}">
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
  }

  .menubar-trigger {
    display: flex;
    align-items: center;
    height: 24px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    transition: background-color 100ms ease, color 100ms ease;
  }

  .menubar-trigger:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }

  :global(.menubar-content) {
    min-width: 160px;
  }
</style>
