<script lang="ts">
  /// About settings page.
  ///
  /// Read-only system info: Arlen version, kernel, daemon statuses. On the
  /// design-system canon: Page/SectionGrid/Group/Row from `@arlen/ui-kit`
  /// (Button stays app-local until the @source consolidation). Stats source is
  /// socket-existence probes (no token-authenticated daemon round-trips).

  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { RefreshCw, ExternalLink, Info, Bug, FileText } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";

  interface DaemonStatus {
    name: string;
    running: boolean;
    probePath: string;
  }

  interface SystemInfo {
    arlenVersion: string | null;
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

  async function openUrl(url: string) {
    try {
      await invoke("open_url", { url });
    } catch (e) {
      console.warn(`open_url(${url}) failed:`, e);
    }
  }
</script>

<Page
  title="About"
  description="System information and daemon status. Read-only — no settings to change here."
>
  <SectionGrid>
    <Group label="Arlen OS">
      <Row label="Version" id="arlen-version">
        {#snippet control()}
          <span class="meta">{info?.arlenVersion ?? "—"}</span>
        {/snippet}
      </Row>
      <Row label="Kernel" id="kernel">
        {#snippet control()}
          <span class="meta">{info?.kernel ?? "—"}</span>
        {/snippet}
      </Row>
      <Row label="Wayland display" id="wayland-display">
        {#snippet control()}
          <span class="meta">{info?.waylandDisplay ?? "—"}</span>
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
              <span class="meta" class:on={d.running}>
                {d.running ? "Running" : "Stopped"}
              </span>
            {/snippet}
          </Row>
        {/each}
      {:else}
        <Row label="Loading…" id="daemon-loading">
          {#snippet control()}<span class="meta">…</span>{/snippet}
        </Row>
      {/if}
      <Row label="Refresh" description="Re-poll daemon status." id="about-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={loading} onclick={refresh}>
            <RefreshCw size={14} class={loading ? "about-spin" : ""} />
            Refresh
          </Button>
        {/snippet}
      </Row>
    </Group>

    <Group label="Resources">
      <Row label="Documentation" id="link-docs">
        {#snippet control()}
          <Button variant="outline" size="sm" onclick={() => openUrl("https://github.com/lunaris-sys/docs")}>
            <FileText size={14} />
            Open
            <ExternalLink size={12} />
          </Button>
        {/snippet}
      </Row>
      <Row label="GitHub organisation" id="link-github">
        {#snippet control()}
          <Button variant="outline" size="sm" onclick={() => openUrl("https://github.com/lunaris-sys")}>
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
            onclick={() => openUrl("https://github.com/lunaris-sys/desktop-shell/issues/new")}
          >
            <Bug size={14} />
            Open
            <ExternalLink size={12} />
          </Button>
        {/snippet}
      </Row>
    </Group>
  </SectionGrid>
</Page>

<style>
  .meta {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .meta.on {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  :global(.about-spin) {
    animation: about-spin 0.8s linear infinite;
  }
  @keyframes about-spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
