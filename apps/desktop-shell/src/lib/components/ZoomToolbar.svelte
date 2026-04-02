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

    let incrementOpen = $state(false);
    let movementOpen = $state(false);

    function movementLabel(m: number): string {
        switch (m) {
            case MOVEMENT_CONTINUOUSLY: return "Continuously";
            case MOVEMENT_ON_EDGE: return "On Edge";
            case MOVEMENT_CENTERED: return "Centered";
            default: return "Unknown";
        }
    }

    function formatLevel(level: number): string {
        return `${Math.round(level * 100)}%`;
    }
</script>

{#if $zoom.visible}
    <div class="zoom-toolbar shell-surface">
        <button class="zoom-btn" onclick={zoomDecrease} title="Zoom out">
            <span class="zoom-icon">-</span>
        </button>

        <span class="zoom-level">{formatLevel($zoom.level)}</span>

        <button class="zoom-btn" onclick={zoomIncrease} title="Zoom in">
            <span class="zoom-icon">+</span>
        </button>

        <div class="zoom-separator"></div>

        <div class="zoom-popover-container">
            <button
                class="zoom-btn zoom-text-btn"
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
                onclick={() => { movementOpen = !movementOpen; incrementOpen = false; }}
                title="View movement"
            >
                <span class="zoom-icon">\u{2026}</span>
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
                                <span class="check">\u{2713}</span>
                            {/if}
                            {opt.label}
                        </button>
                    {/each}
                </div>
            {/if}
        </div>

        <div class="zoom-separator"></div>

        <button class="zoom-btn" onclick={zoomClose} title="Close zoom">
            <span class="zoom-icon">\u{2715}</span>
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
        border-radius: 12px;
        border: 1px solid var(--border);
    }

    .zoom-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        border: none;
        border-radius: 6px;
        cursor: pointer;
        background: transparent;
        color: var(--foreground);
        font-size: 16px;
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

    .zoom-icon {
        font-size: 16px;
        line-height: 1;
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
        border-radius: 8px;
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
        cursor: pointer;
        text-align: left;
    }

    .zoom-popover-item:hover {
        background: color-mix(in srgb, var(--foreground) 8%, transparent);
    }

    .zoom-popover-item.active {
        font-weight: 600;
    }

    .check {
        font-size: 12px;
        color: var(--foreground);
    }
</style>
