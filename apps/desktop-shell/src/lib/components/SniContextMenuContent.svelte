<script lang="ts">
  /// Async-loaded context menu content for SNI tray items.
  /// Fetches the com.canonical.dbusmenu layout on mount and renders
  /// via shadcn ContextMenu primitives with submenu support.

  import * as ContextMenu from "$lib/components/ui/context-menu/index.js";
  import { Check } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";

  interface DbusMenuItem {
    id: number;
    item_type: string;
    label: string;
    enabled: boolean;
    visible: boolean;
    checked: boolean;
    children: DbusMenuItem[];
  }

  interface Props {
    service: string;
    menuPath: string;
  }

  let { service, menuPath }: Props = $props();

  let items = $state<DbusMenuItem[]>([]);
  let loading = $state(true);

  $effect(() => {
    loading = true;
    invoke<DbusMenuItem[]>("get_sni_menu", { service, menuPath })
      .then((result) => { items = result; })
      .catch(() => {})
      .finally(() => { loading = false; });
  });

  async function handleClick(item: DbusMenuItem) {
    if (!item.enabled) return;
    try {
      await invoke("click_sni_menu_item", { service, menuPath, itemId: item.id });
    } catch {}
  }
</script>

{#snippet menuItem(item: DbusMenuItem)}
  {#if !item.visible}
    <!-- hidden -->
  {:else if item.item_type === "separator"}
    <ContextMenu.Separator />
  {:else if item.children.length > 0}
    <ContextMenu.Sub>
      <ContextMenu.SubTrigger disabled={!item.enabled}>
        {#if item.checked}
          <Check class="mr-2 h-4 w-4" />
        {/if}
        {item.label}
      </ContextMenu.SubTrigger>
      <ContextMenu.SubContent class="shell-popover">
        {#each item.children.filter((c) => c.visible) as child}
          {@render menuItem(child)}
        {/each}
      </ContextMenu.SubContent>
    </ContextMenu.Sub>
  {:else}
    <ContextMenu.Item disabled={!item.enabled} onclick={() => handleClick(item)}>
      {#if item.checked}
        <Check class="mr-2 h-4 w-4" />
      {/if}
      {item.label}
    </ContextMenu.Item>
  {/if}
{/snippet}

<ContextMenu.Content class="shell-popover min-w-[180px]">
  {#if loading}
    <ContextMenu.Item disabled>Loading...</ContextMenu.Item>
  {:else if items.length === 0}
    <ContextMenu.Item disabled>No actions</ContextMenu.Item>
  {:else}
    {#each items.filter((i) => i.visible) as item}
      {@render menuItem(item)}
    {/each}
  {/if}
</ContextMenu.Content>
