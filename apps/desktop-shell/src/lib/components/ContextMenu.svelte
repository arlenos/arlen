<script lang="ts">
    import {
        contextMenu,
        activateItem,
        dismissMenu,
        buildMenuTree,
        type MenuItemNode,
    } from "$lib/stores/contextMenu.js";
    import { ChevronRight } from "lucide-svelte";

    /// Tree view derived from the flat DFS stream. Rebuilt whenever the
    /// compositor pushes a new item list via `context-menu-show`.
    const tree = $derived(buildMenuTree($contextMenu.items));

    /// Set of submenu header indices currently open. A header stays open
    /// while the pointer is over either its trigger row or its fly-out.
    let openSubmenus = $state(new Set<number>());

    /// Timer id per submenu header; cleared if the pointer re-enters the
    /// wrap before the delay elapses. Prevents the submenu from vanishing
    /// mid-click when the pointer traces a diagonal path.
    const closeTimers = new Map<number, number>();

    function resetSubmenuState() {
        for (const t of closeTimers.values()) clearTimeout(t);
        closeTimers.clear();
        if (openSubmenus.size > 0) openSubmenus = new Set();
    }

    /// Reset the submenu fly-out state every time the compositor hands us
    /// a new menu instance (or closes the current one). Without this, the
    /// "Move to Workspace" submenu stays open the next time the user
    /// right-clicks — because `openSubmenus` is local `$state` that
    /// outlives the menu's visibility toggle.
    let lastSeenMenuId = $state<number | null>(null);
    $effect(() => {
        const visible = $contextMenu.visible;
        const id = $contextMenu.menu_id;
        if (!visible || id !== lastSeenMenuId) {
            resetSubmenuState();
            lastSeenMenuId = visible ? id : null;
        }
    });

    /// Menu id of the most recently activated menu. Used as a re-entry
    /// guard so a single leaf click never dispatches twice — e.g. when
    /// the browser fires both `pointerup` and a synthetic `click`, or
    /// when Wayland pointer-grab teardown replays the release event.
    let lastActivatedMenuId: number | null = null;

    function openSub(node: MenuItemNode) {
        if (!node.has_submenu || node.disabled) return;
        const t = closeTimers.get(node.index);
        if (t !== undefined) {
            clearTimeout(t);
            closeTimers.delete(node.index);
        }
        openSubmenus.add(node.index);
        openSubmenus = new Set(openSubmenus);
    }

    function closeSubLater(node: MenuItemNode) {
        const existing = closeTimers.get(node.index);
        if (existing !== undefined) clearTimeout(existing);
        const t = window.setTimeout(() => {
            openSubmenus.delete(node.index);
            openSubmenus = new Set(openSubmenus);
            closeTimers.delete(node.index);
        }, 120);
        closeTimers.set(node.index, t);
    }

    /// Handle a leaf-item activation. Idempotent per menu instance: only
    /// the first call for a given menu_id dispatches the Tauri invoke.
    function onActivate(e: Event, index: number) {
        e.preventDefault();
        e.stopPropagation();
        const id = $contextMenu.menu_id;
        if (lastActivatedMenuId === id) return;
        lastActivatedMenuId = id;
        console.log(
            `[contextMenu] activate menu_id=${id} index=${index}`,
        );
        activateItem(id, index);
    }

    /// Click-outside: clicking anywhere on the backdrop (full viewport,
    /// below the menu) tells the compositor to dismiss the menu and
    /// release its pointer grab. Without this the only way to close was
    /// to pick an item or press Escape.
    function onBackdropClick(e: MouseEvent) {
        e.preventDefault();
        e.stopPropagation();
        dismissMenu($contextMenu.menu_id);
    }
</script>

<!--
    The menu renders on top of a full-viewport backdrop that handles
    click-outside → dismiss. The compositor's MenuGrab forwards pointer
    events to the desktop-shell layer surface while the menu is active,
    so the backdrop receives the click normally.

    Submenus render as hover-activated fly-outs to the right of the parent
    row. Tree structure comes from `parent_index` + `has_submenu` in the
    `lunaris-shell-overlay-v1` protocol.
-->
{#snippet menuLevel(nodes: MenuItemNode[])}
    {#each nodes as node (node.index)}
        {#if node.kind === "separator"}
            <div class="ctx-sep"></div>
        {:else if node.has_submenu}
            <div
                class="ctx-sub-wrap"
                role="none"
                onpointerenter={() => openSub(node)}
                onpointerleave={() => closeSubLater(node)}
            >
                <button
                    type="button"
                    role="menuitem"
                    aria-haspopup="menu"
                    aria-expanded={openSubmenus.has(node.index)}
                    class="ctx-item"
                    class:disabled={node.disabled}
                    disabled={node.disabled ?? false}
                >
                    <span>{node.label}</span>
                    <ChevronRight size={12} class="ctx-chevron" />
                </button>
                {#if openSubmenus.has(node.index) && !node.disabled}
                    <div class="ctx-submenu shell-popover" role="menu">
                        {@render menuLevel(node.children)}
                    </div>
                {/if}
            </div>
        {:else}
            <button
                type="button"
                role="menuitem"
                class="ctx-item"
                class:disabled={node.disabled}
                disabled={node.disabled ?? false}
                onclick={(e) => {
                    if (node.disabled) return;
                    onActivate(e, node.index);
                }}
            >
                <span>{node.label}</span>
                {#if node.shortcut}
                    <span class="ctx-shortcut">{node.shortcut}</span>
                {/if}
            </button>
        {/if}
    {/each}
{/snippet}

{#if $contextMenu.visible}
<div
    class="ctx-backdrop"
    role="presentation"
    onmousedown={onBackdropClick}
></div>
<div
    role="menu"
    class="ctx-menu shell-popover"
    style="left: {Math.min($contextMenu.x, window.innerWidth - 200)}px; top: {Math.min($contextMenu.y, window.innerHeight - 100)}px;"
>
    {@render menuLevel(tree)}
</div>
{/if}

<style>
    .ctx-backdrop {
        position: fixed;
        inset: 0;
        z-index: 9998;
        background: transparent;
    }
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
    .ctx-sub-wrap {
        position: relative;
    }
    .ctx-submenu {
        position: absolute;
        left: 100%;
        top: -5px;
        background: var(--color-bg-shell);
        border: 1px solid var(--color-border-default, color-mix(in srgb, var(--color-fg-shell) 20%, transparent));
        border-radius: var(--radius-md, 8px);
        box-shadow: var(--shadow-md);
        padding: 4px 0;
        min-width: 160px;
        z-index: 10000;
    }
    :global(.ctx-chevron) {
        opacity: 0.6;
        margin-left: 8px;
    }
</style>
