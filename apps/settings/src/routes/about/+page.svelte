<script lang="ts">
  /// About settings page (Sprint D).
  ///
  /// Read-only system info: Lunaris version, kernel, daemon
  /// statuses. Stats source mirrors the Knowledge-Graph page —
  /// socket-existence probes, no token-authenticated daemon round-
  /// trips. A "Refresh" button re-polls.

  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { RefreshCw, ExternalLink, Info, Bug, FileText } from "lucide-svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Button } from "$lib/components/ui/button";

  interface DaemonStatus {
    name: string;
    running: boolean;
    probePath: string;
  }

  interface SystemInfo {
    lunarisVersion: string | null;
    kernel: string | null;
    waylandDisplay: string | null;
    daemons: DaemonStatus[];
  }

  let info = $state<SystemInfo | null>(null);
  let loading = $state(false);

  async function refresh() {
    loading = true;
    try {
      info = await invoke<SystemInfo>("about_get_system_info");
    } catch (e) {
      console.warn("about_get_system_info failed:", e);
    } finally {
      loading = false;
    }
  }

  onMount(refresh);

  /// Open an http(s) URL via the backend's xdg-open wrapper. The
  /// previous `window.__TAURI__.opener?.openUrl?.(url)` pattern
  /// silently no-oped because no opener-plugin is installed
  /// (Codex Sprint D review MEDIUM 2). Errors get logged + the
  /// caller knows because we don't catch silently.
  async function openUrl(url: string) {
    try {
      await invoke("open_url", { url });
    } catch (e) {
      console.warn(`open_url(${url}) failed:`, e);
    }
  }
</script>

<SettingsPage
  title="About"
  description="System information and daemon status. Read-only — no settings to change here."
>
  <Group label="Lunaris OS">
    <Row label="Version" id="lunaris-version">
      {#snippet control()}
        <span class="meta-text">
          {info?.lunarisVersion ?? "—"}
        </span>
      {/snippet}
    </Row>

    <Row label="Kernel" id="kernel">
      {#snippet control()}
        <span class="meta-text">{info?.kernel ?? "—"}</span>
      {/snippet}
    </Row>

    <Row label="Wayland display" id="wayland-display">
      {#snippet control()}
        <span class="meta-text">{info?.waylandDisplay ?? "—"}</span>
      {/snippet}
    </Row>
  </Group>

  <Group label="Daemons">
    {#if info}
      {#each info.daemons as d (d.name)}
        <Row
          label={d.name}
          description={d.probePath}
          id={`daemon-${d.name.toLowerCase().replaceAll(' ', '-')}`}
        >
          {#snippet control()}
            <span class="meta-text" class:positive={d.running}>
              {d.running ? "Running ✓" : "Stopped"}
            </span>
          {/snippet}
        </Row>
      {/each}
    {:else}
      <Row label="Loading..." id="daemon-loading">
        {#snippet control()}<span class="meta-text">…</span>{/snippet}
      </Row>
    {/if}

    <div class="footer-row">
      <Button variant="ghost" size="sm" disabled={loading} onclick={refresh}>
        <RefreshCw size={14} class={loading ? "spin" : ""} />
        Refresh
      </Button>
    </div>
  </Group>

  <Group label="Resources">
    <Row label="Documentation" id="link-docs">
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() =>
            openUrl("https://github.com/lunaris-sys/docs")}
        >
          <FileText size={14} />
          Open
          <ExternalLink size={12} />
        </Button>
      {/snippet}
    </Row>

    <Row label="GitHub organisation" id="link-github">
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() => openUrl("https://github.com/lunaris-sys")}
        >
          <Info size={14} />
          Open
          <ExternalLink size={12} />
        </Button>
      {/snippet}
    </Row>

    <Row label="Report an issue" id="link-issues">
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() =>
            openUrl(
              "https://github.com/lunaris-sys/desktop-shell/issues/new",
            )}
        >
          <Bug size={14} />
          Open
          <ExternalLink size={12} />
        </Button>
      {/snippet}
    </Row>
  </Group>
</SettingsPage>

<style>
  .meta-text {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }

  .meta-text.positive {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }

  .footer-row {
    display: flex;
    justify-content: flex-end;
    padding: 0.5rem 1rem;
  }

  .footer-row :global(.spin) {
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
