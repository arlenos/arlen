<script lang="ts">
    import { tabBars, activateTab } from "$lib/stores/tabBars";
</script>

{#each [...$tabBars.values()] as bar (bar.stack_id)}
    {#if bar.tabs.length > 0 && bar.width > 0}
        <div
            class="tab-bar shell-surface"
            style="
                position: fixed;
                left: {bar.x}px;
                top: {bar.y}px;
                width: {bar.width}px;
                height: {bar.height}px;
                z-index: 8000;
            "
        >
            {#each bar.tabs as tab (tab.index)}
                <button
                    class="tab"
                    class:active={tab.index === bar.active}
                    onclick={() => activateTab(bar.stack_id, tab.index)}
                    title={tab.title}
                >
                    <span class="tab-title">{tab.title}</span>
                </button>
            {/each}
        </div>
    {/if}
{/each}

<style>
    .tab-bar {
        display: flex;
        align-items: stretch;
        overflow: hidden;
        gap: 0;
    }

    .tab {
        flex: 1 1 0;
        min-width: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 0 8px;
        border: none;
        cursor: pointer;
        font-size: 12px;
        transition: background-color var(--duration-fast) var(--easing-default);
        background: color-mix(in srgb, var(--background) 60%, transparent);
        color: var(--muted-foreground);
    }

    .tab:hover {
        background: color-mix(in srgb, var(--background) 80%, white 20%);
    }

    .tab.active {
        background: var(--background);
        color: var(--foreground);
        font-weight: 600;
    }

    .tab-title {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
</style>
