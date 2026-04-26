<script lang="ts">
  /// Renders Tier 2 module indicators in the top bar .slot-temp,
  /// sourced from `lunaris-modulesd::ListModules`.
  ///
  /// Replaces the Phase 7 `ModuleIndicatorSlot` which read its list
  /// from the in-process `extension_registry` and used file:// iframes
  /// without nonce binding or CSP enforcement.

  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import SandboxedModuleHost from "$lib/modules/SandboxedModuleHost.svelte";

  interface UiModule {
    id: string;
    name: string;
    version: string;
    tier: string;
    enabled: boolean;
    failed: boolean;
    priority: number;
    extensionPoints: string[];
  }

  let modules = $state<UiModule[]>([]);

  async function refresh() {
    try {
      const list = await invoke<UiModule[]>("modulesd_list_modules");
      modules = list
        .filter(
          (m) =>
            m.tier === "iframe" &&
            m.extensionPoints.includes("topbar") &&
            m.enabled,
        )
        .sort((a, b) => a.priority - b.priority);
    } catch {
      modules = [];
    }
  }

  onMount(() => {
    refresh();
    // Re-pull on a slow tick. Daemon will eventually emit lifecycle
    // events we can subscribe to (Subscribe request); until that's
    // wired, this 30 s refresh keeps Settings ↔ Topbar consistent.
    const handle = setInterval(refresh, 30_000);
    return () => clearInterval(handle);
  });
</script>

{#each modules as mod (mod.id)}
  <div class="mod-indicator" title={mod.name}>
    <SandboxedModuleHost module={mod} slot="topbar" />
  </div>
{/each}

<style>
  .mod-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .mod-indicator :global(iframe) {
    width: 28px;
    height: 28px;
    pointer-events: auto;
  }
</style>
