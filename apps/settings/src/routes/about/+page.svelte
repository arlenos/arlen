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
  import { t } from "$lib/i18n/messages";

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
  title={$t("s.about.title")}
  description={$t("s.about.desc")}
>
  <SectionGrid>
    <Group label="Arlen OS">
      <Row label={$t("s.about.version")} id="arlen-version">
        {#snippet control()}
          <span class="meta">{info?.arlenVersion ?? $t("s.about.unknown")}</span>
        {/snippet}
      </Row>
      <Row label={$t("s.about.kernel")} id="kernel">
        {#snippet control()}
          <span class="meta">{info?.kernel ?? $t("s.about.unknown")}</span>
        {/snippet}
      </Row>
      <Row label={$t("s.about.wayland")} id="wayland-display">
        {#snippet control()}
          <span class="meta">{info?.waylandDisplay ?? $t("s.about.unknown")}</span>
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.about.services")}>
      {#if info}
        {#each info.daemons as d (d.name)}
          <Row
            label={d.name}
            description={d.probePath}
            id={`daemon-${d.name.toLowerCase().replaceAll(' ', '-')}`}
          >
            {#snippet control()}
              <span class="meta" class:on={d.running}>
                {d.running ? $t("s.about.running") : $t("s.about.stopped")}
              </span>
            {/snippet}
          </Row>
        {/each}
      {:else}
        <Row label={$t("s.about.loading")} id="daemon-loading">
          {#snippet control()}<span class="meta">…</span>{/snippet}
        </Row>
      {/if}
      <Row label={$t("s.about.refresh")} description={$t("s.about.refreshDesc")} id="about-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={loading} onclick={refresh}>
            <RefreshCw size={14} class={loading ? "about-spin" : ""} />
            {$t("s.about.refresh")}
          </Button>
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.about.resources")}>
      <Row label={$t("s.about.docs")} id="link-docs">
        {#snippet control()}
          <Button variant="outline" size="sm" onclick={() => openUrl("https://github.com/arlenos/docs")}>
            <FileText size={14} />
            {$t("s.about.open")}
            <ExternalLink size={12} />
          </Button>
        {/snippet}
      </Row>
      <Row label={$t("s.about.github")} id="link-github">
        {#snippet control()}
          <Button variant="outline" size="sm" onclick={() => openUrl("https://github.com/arlenos")}>
            <Info size={14} />
            {$t("s.about.open")}
            <ExternalLink size={12} />
          </Button>
        {/snippet}
      </Row>
      <Row label={$t("s.about.reportIssue")} id="link-issues">
        {#snippet control()}
          <Button
            variant="outline"
            size="sm"
            onclick={() => openUrl("https://github.com/arlenos/desktop-shell/issues/new")}
          >
            <Bug size={14} />
            {$t("s.about.open")}
            <ExternalLink size={12} />
          </Button>
        {/snippet}
      </Row>
    </Group>
  </SectionGrid>
</Page>

<style>
  .meta {
    font-size: var(--text-sm);
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
