<script lang="ts">
    import { contextMenu, activateItem, dismissMenu } from "$lib/stores/contextMenu.js";
</script>

<!--
    No click-outside overlay: the compositor's MenuGrab owns pointer events while active.
    The menu closes via context_menu_closed (Escape / compositor-initiated close) which
    emits lunaris://context-menu-hide -> contextMenu.set(HIDDEN).

    Coordinates are clamped to the viewport because in Phase 2C desktop-shell is a normal
    800x600 window and the compositor sends global coordinates. In Phase 3 (layer-shell,
    full-screen), the coordinates will map directly to the viewport.
-->
{#if $contextMenu.visible}
<div
    role="menu"
    class="ctx-menu shell-popover"
    style="left: {Math.min($contextMenu.x, window.innerWidth - 200)}px; top: {Math.min($contextMenu.y, window.innerHeight - 100)}px;"
>
    {#each $contextMenu.items as item (item.index)}
        {#if item.kind === "separator"}
            <div class="ctx-sep"></div>
        {:else}
            <button
                role="menuitem"
                class="ctx-item"
                class:disabled={item.disabled}
                disabled={item.disabled ?? false}
                onclick={() => activateItem($contextMenu.menu_id, item.index)}
            >
                <span>{item.label}</span>
                {#if item.shortcut}
                    <span class="ctx-shortcut">{item.shortcut}</span>
                {/if}
            </button>
        {/if}
    {/each}
</div>
{/if}

<style>
    .ctx-menu {
        position: fixed;
        z-index: 9999;
        background: var(--color-bg-shell);
        border: 1px solid var(--color-border-default, color-mix(in srgb, var(--color-fg-shell) 20%, transparent));
        border-radius: var(--radius-md, 8px);
        box-shadow: var(--shadow-md);
        padding: 4px 0;
        min-width: 160px;
        color: var(--color-fg-primary, var(--color-fg-shell));
    }
    .ctx-sep {
        height: 1px;
        margin: 4px 0;
        background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    }
    .ctx-item {
        display: flex;
        width: 100%;
        align-items: center;
        justify-content: space-between;
        padding: 6px 12px;
        background: none;
        border: none;
        cursor: pointer;
        font-size: 0.875rem;
        text-align: left;
        color: var(--color-fg-primary, var(--color-fg-shell));
        border-radius: 0;
        transition: background-color var(--duration-fast, 100ms) ease;
    }
    .ctx-item:hover:not(.disabled) {
        background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    }
    .ctx-item.disabled {
        color: var(--color-fg-disabled, color-mix(in srgb, var(--color-fg-shell) 40%, transparent));
        cursor: default;
    }
    .ctx-shortcut {
        font-size: 0.75rem;
        color: var(--color-fg-secondary, color-mix(in srgb, var(--color-fg-shell) 50%, transparent));
        margin-left: 24px;
    }
</style>
