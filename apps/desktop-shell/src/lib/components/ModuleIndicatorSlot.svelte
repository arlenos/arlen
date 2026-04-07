<script lang="ts">
  /// Renders third-party module indicators in the top bar .slot-temp.
  ///
  /// Fetches registered topbar.indicator extensions from the ExtensionRegistry
  /// and renders each via ModuleHost (sandboxed for third-party modules).

  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { ModuleHost } from "$lib/modules/index.js";

  interface TopbarIndicatorInfo {
    module_id: string;
    module_name: string;
    slot: string;
    order: number;
    polling_interval: number;
    priority: number;
  }

  let indicators = $state<TopbarIndicatorInfo[]>([]);

  onMount(async () => {
    try {
      indicators = await invoke<TopbarIndicatorInfo[]>("get_topbar_indicators");
    } catch {}
  });

  // Build module info for ModuleHost from indicator data.
  function moduleInfo(ind: TopbarIndicatorInfo) {
    return {
      id: ind.module_id,
      name: ind.module_name,
      module_type: ind.priority >= 100 ? "third-party" : "system",
      path: "", // Resolved by ModuleHost from module loader
    };
  }
</script>

{#each indicators as ind (ind.module_id)}
  <div class="mod-indicator" title={ind.module_name}>
    <ModuleHost
      module={moduleInfo(ind)}
      extensionPoint="topbar.indicator"
    />
  </div>
{/each}

<style>
  .mod-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 4px;
    overflow: hidden;
  }
  .mod-indicator :global(iframe) {
    width: 28px;
    height: 28px;
    pointer-events: auto;
  }
</style>
