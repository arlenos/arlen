<script lang="ts">
  /// Module management panel: list, enable/disable, error status.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { Switch } from "$lib/components/ui/switch/index.js";
  import {
    Puzzle, AlertTriangle, RefreshCw,
    ChevronDown, ChevronUp,
  } from "lucide-svelte";

  interface ModuleSummary {
    id: string;
    name: string;
    version: string;
    description: string;
    module_type: string;
    source: "system" | "user";
    enabled: boolean;
    has_waypointer: boolean;
    has_topbar: boolean;
    has_settings: boolean;
    icon: string;
  }

  interface ModuleErrorStatus {
    module_id: string;
    error_count: number;
    last_error: string;
    auto_disabled: boolean;
  }

  let modules = $state<ModuleSummary[]>([]);
  let errors = $state<ModuleErrorStatus[]>([]);
  let expandedId = $state<string | null>(null);
  let loading = $state(true);

  async function loadModules() {
    loading = true;
    try {
      modules = await invoke<ModuleSummary[]>("list_modules");
      errors = await invoke<ModuleErrorStatus[]>("get_module_errors");
    } catch {}
    loading = false;
  }

  onMount(() => {
    loadModules();
    const unlisten = listen("lunaris://module-auto-disabled", () => loadModules());
    return () => { unlisten.then((fn) => fn()); };
  });

  async function toggleModule(id: string, enabled: boolean) {
    try {
      await invoke("set_module_enabled", { id, enabled });
      if (enabled) {
        await invoke("reset_module_errors", { moduleId: id });
      }
      await loadModules();
    } catch {}
  }

  async function reEnable(id: string) {
    try {
      await invoke("reset_module_errors", { moduleId: id });
      await invoke("set_module_enabled", { id, enabled: true });
      await loadModules();
    } catch {}
  }

  function getError(id: string): ModuleErrorStatus | undefined {
    return errors.find((e) => e.module_id === id);
  }

  function extensionLabels(m: ModuleSummary): string[] {
    const labels: string[] = [];
    if (m.has_waypointer) labels.push("Waypointer");
    if (m.has_topbar) labels.push("Top Bar");
    if (m.has_settings) labels.push("Settings");
    return labels;
  }

  function typeLabel(t: string): string {
    if (t === "system") return "System";
    if (t === "first-party") return "First Party";
    return "Third Party";
  }
</script>

<div class="mod-panel">
  <div class="mod-header">
    <Puzzle size={20} strokeWidth={1.5} />
    <h2>Extensions</h2>
  </div>

  {#if loading}
    <div class="mod-empty">Loading...</div>
  {:else if modules.length === 0}
    <div class="mod-empty">No modules installed</div>
  {:else}
    <div class="mod-list">
      {#each modules as mod_item}
        {@const err = getError(mod_item.id)}
        {@const isAutoDisabled = err?.auto_disabled ?? false}
        <div class="mod-card" class:disabled={!mod_item.enabled} class:error={isAutoDisabled}>

          <div class="mod-row">
            <div class="mod-info">
              <span class="mod-name">{mod_item.name}</span>
              <span class="mod-version">v{mod_item.version}</span>
              <span class="mod-badge" class:system={mod_item.module_type === "system"} class:first-party={mod_item.module_type === "first-party"}>
                {typeLabel(mod_item.module_type)}
              </span>
              <span class="mod-badge source">{mod_item.source}</span>
            </div>

            <div class="mod-actions">
              {#if isAutoDisabled}
                <button class="mod-reenable" onclick={() => reEnable(mod_item.id)}>
                  <RefreshCw size={12} strokeWidth={2} />
                  Re-enable
                </button>
              {:else}
                <Switch
                  checked={mod_item.enabled}
                  onCheckedChange={() => toggleModule(mod_item.id, !mod_item.enabled)}
                />
              {/if}

              <button
                class="mod-expand"
                onclick={() => expandedId = expandedId === mod_item.id ? null : mod_item.id}
              >
                {#if expandedId === mod_item.id}
                  <ChevronUp size={14} strokeWidth={1.5} />
                {:else}
                  <ChevronDown size={14} strokeWidth={1.5} />
                {/if}
              </button>
            </div>
          </div>

          {#if isAutoDisabled}
            <div class="mod-warning">
              <AlertTriangle size={12} strokeWidth={2} />
              <span>Auto-disabled: {err?.error_count} errors. Last: {err?.last_error}</span>
            </div>
          {/if}

          {#if expandedId === mod_item.id}
            <div class="mod-detail">
              <div class="mod-id">{mod_item.id}</div>
              {#if mod_item.description}
                <div class="mod-desc">{mod_item.description}</div>
              {/if}
              {#if extensionLabels(mod_item).length > 0}
                <div class="mod-extensions">
                  {#each extensionLabels(mod_item) as ext}
                    <span class="mod-ext-chip">{ext}</span>
                  {/each}
                </div>
              {/if}
              {#if err && err.error_count > 0}
                <div class="mod-error-info">
                  {err.error_count} errors | Last: {err.last_error}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .mod-panel { padding: 16px; color: var(--color-fg-primary, #fafafa); }
  .mod-header { display: flex; align-items: center; gap: 10px; margin-bottom: 16px; }
  .mod-header h2 { font-size: 1rem; font-weight: 600; margin: 0; }
  .mod-empty { text-align: center; padding: 32px; opacity: 0.4; font-size: 0.875rem; }

  .mod-list { display: flex; flex-direction: column; gap: 4px; }

  .mod-card {
    border-radius: var(--radius-md, 8px); padding: 8px 12px;
    transition: background-color var(--duration-fast, 100ms) ease;
  }
  .mod-card:hover { background: color-mix(in srgb, var(--color-fg-shell) 5%, transparent); }
  .mod-card.disabled { opacity: 0.6; }
  .mod-card.error { border-left: 3px solid var(--color-error); }

  .mod-row { display: flex; align-items: center; gap: 8px; }
  .mod-info { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; flex-wrap: wrap; }
  .mod-name { font-size: 0.8125rem; font-weight: 500; }
  .mod-version { font-size: 0.6875rem; opacity: 0.5; }

  .mod-badge {
    font-size: 0.5625rem; font-weight: 600; text-transform: uppercase;
    padding: 1px 5px; border-radius: var(--radius-sm, 4px);
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
  }
  .mod-badge.system { background: color-mix(in srgb, var(--color-info) 20%, transparent); color: var(--color-info); }
  .mod-badge.first-party { background: color-mix(in srgb, var(--color-success) 20%, transparent); color: var(--color-success); }
  .mod-badge.source { background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent); opacity: 0.6; }

  .mod-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .mod-expand {
    width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: var(--radius-sm, 4px);
    color: var(--color-fg-shell); cursor: pointer; padding: 0; opacity: 0.5;
  }
  .mod-expand:hover { opacity: 1; background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }

  .mod-reenable {
    display: flex; align-items: center; gap: 4px; padding: 3px 8px;
    font-size: 0.6875rem; font-weight: 500;
    background: color-mix(in srgb, var(--color-warning) 15%, transparent);
    color: var(--color-warning); border: none; border-radius: var(--radius-sm, 4px);
    cursor: pointer;
  }
  .mod-reenable:hover { background: color-mix(in srgb, var(--color-warning) 25%, transparent); }

  .mod-warning {
    display: flex; align-items: center; gap: 6px; margin-top: 4px;
    font-size: 0.6875rem; color: var(--color-error); opacity: 0.9;
  }

  .mod-detail {
    margin-top: 8px; padding-top: 8px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    display: flex; flex-direction: column; gap: 4px;
  }
  .mod-id { font-size: 0.6875rem; font-family: monospace; opacity: 0.5; }
  .mod-desc { font-size: 0.75rem; opacity: 0.7; }
  .mod-extensions { display: flex; gap: 4px; flex-wrap: wrap; }
  .mod-ext-chip {
    font-size: 0.625rem; padding: 1px 6px;
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    color: var(--color-accent); border-radius: var(--radius-sm, 4px);
  }
  .mod-error-info { font-size: 0.6875rem; color: var(--color-error); opacity: 0.8; }
</style>
