<script lang="ts">
    import { contextMenu, activateItem, dismissMenu } from "$lib/stores/contextMenu.js";
</script>

<!--
    No click-outside overlay: the compositor's MenuGrab owns pointer events while active.
    The menu closes via context_menu_closed (Escape / compositor-initiated close) which
    emits lunaris://context-menu-hide → contextMenu.set(HIDDEN).

    Coordinates are clamped to the viewport because in Phase 2C desktop-shell is a normal
    800x600 window and the compositor sends global coordinates. In Phase 3 (layer-shell,
    full-screen), the coordinates will map directly to the viewport.
-->
{#if $contextMenu.visible}
<div
    role="menu"
    style="
        position: fixed;
        left: {Math.min($contextMenu.x, window.innerWidth - 200)}px;
        top: {Math.min($contextMenu.y, window.innerHeight - 100)}px;
        z-index: 9999;
        background: white;
        border: 1px solid #ccc;
        border-radius: 4px;
        box-shadow: 0 4px 12px rgba(0,0,0,0.15);
        padding: 4px 0;
        min-width: 160px;
    "
>
    {#each $contextMenu.items as item (item.index)}
        {#if item.kind === "separator"}
            <hr style="margin: 4px 0; border: none; border-top: 1px solid #e5e5e5;" />
        {:else}
            <button
                role="menuitem"
                disabled={item.disabled ?? false}
                onclick={() => activateItem($contextMenu.menu_id, item.index)}
                style="
                    display: flex;
                    width: 100%;
                    align-items: center;
                    justify-content: space-between;
                    padding: 6px 12px;
                    background: none;
                    border: none;
                    cursor: {item.disabled ? 'default' : 'pointer'};
                    font-size: 14px;
                    text-align: left;
                    color: {item.disabled ? '#aaa' : '#111'};
                "
            >
                <span>{item.label}</span>
                {#if item.shortcut}
                    <span style="font-size: 12px; color: #888; margin-left: 24px;">{item.shortcut}</span>
                {/if}
            </button>
        {/if}
    {/each}
</div>
{/if}
