<script lang="ts">
    import { contextMenu, activateItem, dismissMenu } from "$lib/stores/contextMenu.js";
    import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
</script>

{#if $contextMenu.visible}
<div
    style="position: fixed; left: {$contextMenu.x}px; top: {$contextMenu.y}px; width: 0; height: 0;"
>
    <DropdownMenu.Root
        open={true}
        onOpenChange={(open) => {
            if (!open && $contextMenu.menu_id !== 0) {
                dismissMenu($contextMenu.menu_id);
            }
        }}
    >
        <DropdownMenu.Trigger class="size-0 opacity-0 pointer-events-none" />
        <DropdownMenu.Content class="shell-surface min-w-48">
            {#each $contextMenu.items as item (item.index)}
                {#if item.kind === "separator"}
                    <DropdownMenu.Separator />
                {:else if item.toggled}
                    <DropdownMenu.CheckboxItem
                        checked={item.toggled ?? false}
                        disabled={item.disabled ?? false}
                        onclick={() => activateItem($contextMenu.menu_id, item.index)}
                    >
                        {item.label}
                        {#if item.shortcut}
                            <DropdownMenu.Shortcut>{item.shortcut}</DropdownMenu.Shortcut>
                        {/if}
                    </DropdownMenu.CheckboxItem>
                {:else}
                    <DropdownMenu.Item
                        disabled={item.disabled ?? false}
                        onclick={() => activateItem($contextMenu.menu_id, item.index)}
                    >
                        {item.label}
                        {#if item.shortcut}
                            <DropdownMenu.Shortcut>{item.shortcut}</DropdownMenu.Shortcut>
                        {/if}
                    </DropdownMenu.Item>
                {/if}
            {/each}
        </DropdownMenu.Content>
    </DropdownMenu.Root>
</div>
{/if}
