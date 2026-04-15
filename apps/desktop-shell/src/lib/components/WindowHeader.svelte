<script lang="ts">
    import {
        windowHeaders,
        headerAction,
        HEADER_ACTION_MINIMIZE,
        HEADER_ACTION_MAXIMIZE,
        HEADER_ACTION_CLOSE,
        HEADER_ACTION_MOVE,
    } from "$lib/stores/windowHeaders";
</script>

{#each [...$windowHeaders.values()] as hdr (hdr.surface_id)}
    <div
        class="window-header"
        class:activated={hdr.activated}
        style="
            position: fixed;
            left: {hdr.x}px;
            top: {hdr.y}px;
            width: {hdr.width}px;
            height: {hdr.height}px;
            z-index: 7000;
        "
    >
        <div
            class="header-drag"
            role="button"
            tabindex="-1"
            onmousedown={() => headerAction(hdr.surface_id, HEADER_ACTION_MOVE)}
        >
            <span class="header-title">{hdr.title}</span>
        </div>

        <div class="header-buttons">
            {#if hdr.has_minimize}
                <button
                    class="header-btn minimize"
                    onclick={() => headerAction(hdr.surface_id, HEADER_ACTION_MINIMIZE)}
                    title="Minimize"
                >
                    <span class="btn-icon">&#x2013;</span>
                </button>
            {/if}
            {#if hdr.has_maximize}
                <button
                    class="header-btn maximize"
                    onclick={() => headerAction(hdr.surface_id, HEADER_ACTION_MAXIMIZE)}
                    title="Maximize"
                >
                    <span class="btn-icon">&#x25A1;</span>
                </button>
            {/if}
            <button
                class="header-btn close"
                onclick={() => headerAction(hdr.surface_id, HEADER_ACTION_CLOSE)}
                title="Close"
            >
                <span class="btn-icon">&#x2715;</span>
            </button>
        </div>
    </div>
{/each}

<style>
    .window-header {
        display: flex;
        align-items: center;
        background: var(--background);
        color: var(--muted-foreground);
        border-bottom: 1px solid var(--border);
        border-radius: var(--radius-md) var(--radius-md) 0 0;
        overflow: hidden;
        pointer-events: auto;
    }

    .window-header.activated {
        color: var(--foreground);
    }

    .header-drag {
        flex: 1;
        min-width: 0;
        display: flex;
        align-items: center;
        padding: 0 12px;
        height: 100%;
        cursor: grab;
        user-select: none;
        -webkit-user-select: none;
    }

    .header-drag:active {
        cursor: grabbing;
    }

    .header-title {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: 13px;
        font-weight: 500;
    }

    .header-buttons {
        display: flex;
        align-items: center;
        gap: 0;
        padding-right: 4px;
    }

    .header-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        border: none;
        border-radius: var(--radius-md);
        background: transparent;
        color: inherit;
        cursor: pointer;
        font-size: 14px;
        transition: background var(--duration-fast, 150ms) var(--easing-default, ease);
    }

    .header-btn:hover {
        background: color-mix(in srgb, var(--foreground) 10%, transparent);
    }

    .header-btn.close:hover {
        background: color-mix(in srgb, var(--color-error) 80%, transparent);
        color: var(--color-fg-primary);
    }

    .btn-icon {
        line-height: 1;
    }
</style>
