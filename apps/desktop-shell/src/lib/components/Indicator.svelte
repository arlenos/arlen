<script lang="ts">
    import { indicators, type IndicatorState } from "$lib/stores/indicators";

    const LABELS: Record<number, string> = {
        1: "Stack Windows",
        2: "Swap Windows",
    };

    const ICONS: Record<number, string> = {
        1: "\u{29C9}",
        2: "\u{21C4}",
    };

    function directionLabel(dir: number): string {
        return dir === 1 ? "Grow" : "Shrink";
    }

    function edgeActive(edges: number, bit: number): boolean {
        return (edges & bit) !== 0;
    }
</script>

{#each [...$indicators.values()] as ind (ind.kind)}
    {#if ind.kind === 1 || ind.kind === 2}
        <!-- Stack hover / Swap: centered badge -->
        <div class="indicator-badge shell-surface">
            <span class="indicator-icon">{ICONS[ind.kind]}</span>
            <span class="indicator-label">{LABELS[ind.kind]}</span>
        </div>
    {:else if ind.kind === 3}
        <!-- Resize: edge arrows + shortcut hints -->
        <div class="indicator-resize shell-surface">
            {#if edgeActive(ind.edges, 1)}
                <div class="resize-arrow top">\u{2191}</div>
            {/if}
            {#if edgeActive(ind.edges, 4)}
                <div class="resize-arrow left">\u{2190}</div>
            {/if}
            <div class="resize-center">
                {#if ind.shortcut1}
                    <span class="shortcut">{ind.shortcut1}{directionLabel(1)}</span>
                {/if}
                {#if ind.shortcut2}
                    <span class="shortcut">{ind.shortcut2}{directionLabel(2)}</span>
                {/if}
            </div>
            {#if edgeActive(ind.edges, 8)}
                <div class="resize-arrow right">\u{2192}</div>
            {/if}
            {#if edgeActive(ind.edges, 2)}
                <div class="resize-arrow bottom">\u{2193}</div>
            {/if}
        </div>
    {/if}
{/each}

<style>
    .indicator-badge {
        position: fixed;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%);
        z-index: 9000;
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 16px 24px;
        border-radius: 18px;
        pointer-events: none;
    }

    .indicator-icon {
        font-size: 28px;
    }

    .indicator-label {
        font-size: 18px;
        font-weight: 600;
        color: var(--foreground);
    }

    .indicator-resize {
        position: fixed;
        inset: 0;
        z-index: 9000;
        display: grid;
        grid-template-areas:
            ".    top    ."
            "left center right"
            ".    bottom .";
        grid-template-columns: 48px 1fr 48px;
        grid-template-rows: 48px 1fr 48px;
        pointer-events: none;
    }

    .resize-arrow {
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 24px;
        color: var(--foreground);
        background: var(--background);
        border-radius: 18px;
        width: 36px;
        height: 36px;
    }

    .resize-arrow.top {
        grid-area: top;
        justify-self: center;
    }
    .resize-arrow.bottom {
        grid-area: bottom;
        justify-self: center;
    }
    .resize-arrow.left {
        grid-area: left;
        align-self: center;
    }
    .resize-arrow.right {
        grid-area: right;
        align-self: center;
    }

    .resize-center {
        grid-area: center;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 4px;
        background: var(--background);
        border-radius: 18px;
        padding: 16px;
        align-self: center;
        justify-self: center;
    }

    .shortcut {
        font-size: 14px;
        color: var(--foreground);
    }
</style>
