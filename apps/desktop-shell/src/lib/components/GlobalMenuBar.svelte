<script lang="ts">
  import { activeMenu, activeAppId, dispatchMenuAction, type MenuGroup, type MenuItem } from "$lib/stores/menus.js";
  import { activeAppName } from "$lib/stores/windows.js";
  import {
    Root, Trigger, Content, Item, Separator, CheckboxItem, Shortcut,
  } from "$lib/components/ui/dropdown-menu/index.js";
  const shellColors =
    "bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  function handleAction(action: string) {
    const appId = $activeAppId;
    if (appId) dispatchMenuAction(appId, action);
  }

</script>

<div class="menubar">
  <span class="menubar-appname">
    {$activeAppName || "Lunaris"}
  </span>

  {#if $activeMenu}
    {#each $activeMenu as group}
      <Root>
        <Trigger>
          {#snippet child({ props })}
            <button class="menubar-trigger" {...props}>
              {group.label}
            </button>
          {/snippet}
        </Trigger>
        <Content sideOffset={4} class="menubar-content {shellColors}">
          {#each group.items as item}
            {#if item.type === "separator"}
              <Separator />
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
            {:else if item.type === "submenu"}
              <!-- Submenus rendered as flat items for now -->
              <Item
                disabled={item.disabled}
                onSelect={() => handleAction(item.action)}
              >
                {item.label} &rsaquo;
              </Item>
            {/if}
          {/each}
        </Content>
      </Root>
    {/each}
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
    border-radius: 4px;
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
