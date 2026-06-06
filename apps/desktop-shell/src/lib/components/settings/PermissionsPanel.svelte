<script lang="ts">
  /// Permissions panel: lists apps and their permission scopes.
  /// Currently read-only. Write support requires D-Bus integration
  /// with org.lunaris.PermissionHelper1.

  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { Shield, ShieldCheck, ShieldAlert, ChevronDown, ChevronUp } from "lucide-svelte";
  import PermissionScope from "./PermissionScope.svelte";

  interface AppSummary {
    app_id: string;
    tier: string;
    has_graph: boolean;
    has_network: boolean;
    has_filesystem: boolean;
    has_notifications: boolean;
    has_clipboard: boolean;
    has_background: boolean;
  }

  interface AppDetail {
    app_id: string;
    tier: string;
    graph: { read: string[]; write: string[]; app_isolated: boolean };
    event_bus: { publish: string[]; subscribe: string[] };
    filesystem: { home: boolean; documents: boolean; downloads: boolean; pictures: boolean; music: boolean; videos: boolean; custom: string[] };
    network: { allow_all: boolean; allowed_domains: string[] };
    notifications: boolean;
    clipboard: { read: boolean; write: boolean };
    system: { autostart: boolean; background: boolean };
  }

  let apps = $state<AppSummary[]>([]);
  let expandedApp = $state<string | null>(null);
  let detail = $state<AppDetail | null>(null);
  let loading = $state(true);

  onMount(async () => {
    try {
      apps = await invoke<AppSummary[]>("get_app_permissions");
    } catch {}
    loading = false;
  });

  async function toggleApp(appId: string) {
    if (expandedApp === appId) {
      expandedApp = null;
      detail = null;
      return;
    }
    expandedApp = appId;
    try {
      detail = await invoke<AppDetail>("get_app_permission_detail", { appId });
    } catch {
      detail = null;
    }
  }

  function tierIcon(tier: string) {
    if (tier === "system") return ShieldCheck;
    if (tier === "first-party") return ShieldCheck;
    return ShieldAlert;
  }

  function tierLabel(tier: string): string {
    if (tier === "system") return "System";
    if (tier === "first-party") return "First Party";
    return "Third Party";
  }

  function permissionCount(app: AppSummary): number {
    let c = 0;
    if (app.has_graph) c++;
    if (app.has_network) c++;
    if (app.has_filesystem) c++;
    if (app.has_notifications) c++;
    if (app.has_clipboard) c++;
    if (app.has_background) c++;
    return c;
  }
</script>

<div class="perm-panel">
  <div class="perm-header">
    <Shield size={20} strokeWidth={1.5} />
    <h2>App Permissions</h2>
  </div>

  {#if loading}
    <div class="perm-empty">Loading...</div>
  {:else if apps.length === 0}
    <div class="perm-empty">No permission profiles found</div>
  {:else}
    <div class="perm-list">
      {#each apps as app}
        <div class="perm-app" class:expanded={expandedApp === app.app_id}>
          <button class="perm-app-row" onclick={() => toggleApp(app.app_id)}>
            <span class="perm-tier-badge" class:system={app.tier === "system"} class:first-party={app.tier === "first-party"}>
              {tierLabel(app.tier)}
            </span>
            <span class="perm-app-name">{app.app_id}</span>
            <span class="perm-count">{permissionCount(app)} scopes</span>
            {#if expandedApp === app.app_id}
              <ChevronUp size={14} strokeWidth={1.5} />
            {:else}
              <ChevronDown size={14} strokeWidth={1.5} />
            {/if}
          </button>

          {#if expandedApp === app.app_id && detail}
            <div class="perm-detail">
              {#if detail.graph.read.length > 0 || detail.graph.write.length > 0}
                <PermissionScope
                  title="Knowledge Graph"
                  booleans={[
                    { label: "App Isolated", value: detail.graph.app_isolated },
                  ]}
                />
                {#if detail.graph.read.length > 0}
                  <PermissionScope title="Graph Read" items={detail.graph.read} />
                {/if}
                {#if detail.graph.write.length > 0}
                  <PermissionScope title="Graph Write" items={detail.graph.write} />
                {/if}
              {/if}

              {#if detail.event_bus.publish.length > 0 || detail.event_bus.subscribe.length > 0}
                <PermissionScope title="Event Bus Publish" items={detail.event_bus.publish} />
                <PermissionScope title="Event Bus Subscribe" items={detail.event_bus.subscribe} />
              {/if}

              <PermissionScope
                title="Filesystem"
                booleans={[
                  { label: "Home", value: detail.filesystem.home },
                  { label: "Documents", value: detail.filesystem.documents },
                  { label: "Downloads", value: detail.filesystem.downloads },
                  { label: "Pictures", value: detail.filesystem.pictures },
                  { label: "Music", value: detail.filesystem.music },
                  { label: "Videos", value: detail.filesystem.videos },
                ]}
              />
              {#if detail.filesystem.custom.length > 0}
                <PermissionScope title="Custom Paths" items={detail.filesystem.custom} />
              {/if}

              <PermissionScope
                title="Network"
                booleans={[{ label: "Allow All", value: detail.network.allow_all }]}
              />
              {#if detail.network.allowed_domains.length > 0}
                <PermissionScope title="Allowed Domains" items={detail.network.allowed_domains} />
              {/if}

              <PermissionScope
                title="Other"
                booleans={[
                  { label: "Notifications", value: detail.notifications },
                  { label: "Clipboard Read", value: detail.clipboard.read },
                  { label: "Clipboard Write", value: detail.clipboard.write },
                  { label: "Autostart", value: detail.system.autostart },
                  { label: "Background", value: detail.system.background },
                ]}
              />
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .perm-panel { padding: 16px; color: var(--color-fg-primary, #fafafa); }
  .perm-header { display: flex; align-items: center; gap: 10px; margin-bottom: 16px; }
  .perm-header h2 { font-size: 1rem; font-weight: 600; margin: 0; }
  .perm-empty { text-align: center; padding: 32px; opacity: 0.4; font-size: 0.875rem; }

  .perm-list { display: flex; flex-direction: column; gap: 4px; }
  .perm-app { border-radius: var(--radius-md, 8px); overflow: hidden; }
  .perm-app.expanded { background: color-mix(in srgb, var(--color-fg-shell) 5%, transparent); }

  .perm-app-row {
    display: flex; align-items: center; gap: 10px; width: 100%;
    padding: 10px 12px; background: transparent; border: none;
    color: var(--color-fg-primary, var(--color-fg-shell));
    font-size: 0.8125rem; cursor: pointer; text-align: left;
    border-radius: var(--radius-md, 8px);
    transition: background-color var(--duration-fast, 100ms) ease;
  }
  .perm-app-row:hover { background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent); }

  .perm-tier-badge {
    font-size: 0.625rem; font-weight: 600; text-transform: uppercase;
    padding: 2px 6px; border-radius: var(--radius-sm, 4px);
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    flex-shrink: 0;
  }
  .perm-tier-badge.system { background: color-mix(in srgb, var(--color-info) 20%, transparent); color: var(--color-info); }
  .perm-tier-badge.first-party { background: color-mix(in srgb, var(--color-success) 20%, transparent); color: var(--color-success); }

  .perm-app-name { flex: 1; font-weight: 500; }
  .perm-count { font-size: 0.6875rem; opacity: 0.5; flex-shrink: 0; }

  .perm-detail { padding: 8px 12px 12px; }
</style>
