<script lang="ts">
    import {
        zoom,
        zoomIncrease,
        zoomDecrease,
        zoomClose,
        zoomSetIncrement,
        zoomSetMovement,
        INCREMENTS,
        MOVEMENT_CONTINUOUSLY,
        MOVEMENT_ON_EDGE,
        MOVEMENT_CENTERED,
    } from "$lib/stores/zoom";
    import { Minus, Plus, Ellipsis, Check, X } from "lucide-svelte";

    let incrementOpen = $state(false);
    let movementOpen = $state(false);

    /// Either hand-rolled popover dangles open until its trigger is
    /// re-clicked — close them on any press outside the toolbar and
    /// on Escape, like every other transient surface in the shell.
    function onWindowPointerDown(e: PointerEvent) {
        if (!incrementOpen && !movementOpen) return;
        if (
            e.target instanceof Element &&
            e.target.closest(".zoom-popover-container")
        ) {
            return;
        }
        incrementOpen = false;
        movementOpen = false;
    }

    function onWindowKeydown(e: KeyboardEvent) {
        if (e.key === "Escape" && (incrementOpen || movementOpen)) {
            incrementOpen = false;
            movementOpen = false;
        }
    }

    function formatLevel(level: number): string {
        return `${Math.round(level * 100)}%`;
    }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

{#if $zoom.visible}
    <div class="zoom-toolbar shell-surface">
        <button class="zoom-btn" onclick={zoomDecrease} aria-label="Zoom out">
            <Minus size={14} strokeWidth={2} />
        </button>

        <span class="zoom-level">{formatLevel($zoom.level)}</span>

        <button class="zoom-btn" onclick={zoomIncrease} aria-label="Zoom in">
            <Plus size={14} strokeWidth={2} />
        </button>

        <div class="zoom-separator"></div>

        <div class="zoom-popover-container">
            <button
                class="zoom-btn zoom-text-btn"
                aria-label="Zoom step"
                aria-haspopup="menu"
                aria-expanded={incrementOpen}
                onclick={() => { incrementOpen = !incrementOpen; movementOpen = false; }}
            >
                {$zoom.increment}%
            </button>
            {#if incrementOpen}
                <div class="zoom-popover">
                    {#each INCREMENTS as val}
                        <button
                            class="zoom-popover-item"
                            class:active={val === $zoom.increment}
                            onclick={() => { zoomSetIncrement(val); incrementOpen = false; }}
                        >
                            {val}%
                        </button>
                    {/each}
                </div>
            {/if}
        </div>

        <div class="zoom-popover-container">
            <button
                class="zoom-btn"
                aria-label="Zoom movement"
                aria-haspopup="menu"
                aria-expanded={movementOpen}
                onclick={() => { movementOpen = !movementOpen; incrementOpen = false; }}
            >
                <Ellipsis size={14} strokeWidth={2} />
            </button>
            {#if movementOpen}
                <div class="zoom-popover">
                    {#each [
                        { mode: MOVEMENT_CONTINUOUSLY, label: "Move continuously" },
                        { mode: MOVEMENT_ON_EDGE, label: "Move on edge" },
                        { mode: MOVEMENT_CENTERED, label: "Move centered" },
                    ] as opt}
                        <button
                            class="zoom-popover-item"
                            class:active={opt.mode === $zoom.movement}
                            onclick={() => { zoomSetMovement(opt.mode); movementOpen = false; }}
                        >
                            {#if opt.mode === $zoom.movement}
                                <span class="check"><Check size={12} strokeWidth={2} /></span>
                            {/if}
                            {opt.label}
                        </button>
                    {/each}
                </div>
            {/if}
        </div>

        <div class="zoom-separator"></div>

        <button class="zoom-btn" onclick={zoomClose} aria-label="Turn off zoom">
            <X size={14} strokeWidth={2} />
        </button>
    </div>
{/if}

<style>
    .zoom-toolbar {
        position: fixed;
        bottom: 25%;
        left: 50%;
        transform: translateX(-50%);
        z-index: 9500;
        display: flex;
        align-items: center;
        gap: 4px;
        padding: 6px 10px;
        border-radius: var(--radius-card);
        border: 1px solid var(--border);
    }

    .zoom-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: var(--height-control, 28px);
        height: var(--height-control, 28px);
        border: none;
        border-radius: var(--radius-input);
        background: transparent;
        color: var(--foreground);
        transition: background var(--duration-fast, 150ms) var(--easing-default, ease);
    }

    .zoom-btn:hover {
        background: color-mix(in srgb, var(--foreground) 10%, transparent);
    }

    .zoom-text-btn {
        width: auto;
        padding: 0 8px;
        font-size: 13px;
        font-weight: 500;
    }

    .zoom-level {
        min-width: 48px;
        text-align: center;
        font-size: 13px;
        font-weight: 600;
        color: var(--foreground);
    }

    .zoom-separator {
        width: 1px;
        height: 20px;
        background: var(--border);
        margin: 0 2px;
    }

    .zoom-popover-container {
        position: relative;
    }

    .zoom-popover {
        position: absolute;
        bottom: calc(100% + 8px);
        left: 50%;
        transform: translateX(-50%);
        background: var(--background);
        border: 1px solid var(--border);
        border-radius: var(--radius-input);
        padding: 4px 0;
        min-width: 140px;
        z-index: 9600;
    }

    .zoom-popover-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        padding: 6px 12px;
        border: none;
        background: none;
        color: var(--foreground);
        font-size: 13px;
        text-align: left;
    }

    .zoom-popover-item:hover {
        background: color-mix(in srgb, var(--foreground) 8%, transparent);
    }

    .zoom-popover-item.active {
        font-weight: 600;
    }

    .check {
        display: inline-flex;
        color: var(--color-accent);
    }
</style>
