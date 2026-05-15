<script lang="ts">
  /// AI settings page (Phase 9-α S7).
  ///
  /// Reads/writes `~/.config/lunaris/ai.toml`. The `lunaris-ai-daemon`
  /// watches that file: toggling "Enable AI features" switches the AI
  /// layer on/off live. A provider change needs a daemon restart (the
  /// same convention `graph.toml` uses), so the page surfaces a hint
  /// instead of pretending it is instant.
  ///
  /// Phase 9-alpha basics only: enable toggle, daemon status,
  /// provider selection. The access-tier slider, action-mode
  /// selector, routing editor, and audit-log viewer dock onto this
  /// page in Phase 9-delta S24, once the infrastructure exists.

  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Sparkles, RefreshCw, AlertCircle } from "lucide-svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Switch } from "$lib/components/ui/switch";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import { Button } from "$lib/components/ui/button";
  import { ai } from "$lib/stores/ai";

  interface AiStatus {
    daemonRunning: boolean;
    proxyRunning: boolean;
  }

  /// Phase 9-α ships only the local Ollama provider in the proxy
  /// catalog. Cloud providers join this list in Phase 9-β/γ once
  /// keyring-backed authentication lands.
  const PROVIDERS = [{ value: "ollama-default", label: "Ollama (local)" }];

  let enabled = $state(false);
  let provider = $state("ollama-default");
  /// The provider value the daemon last started with. A mismatch
  /// against `provider` means a restart is pending.
  let providerAtLoad = $state("ollama-default");

  let status = $state<AiStatus | null>(null);
  let statusLoading = $state(false);
  let statusError = $state<string | null>(null);

  async function refreshStatus(): Promise<void> {
    statusLoading = true;
    statusError = null;
    try {
      status = await invoke<AiStatus>("ai_status");
    } catch (e) {
      statusError = String(e);
      status = null;
    } finally {
      statusLoading = false;
    }
  }

  onMount(async () => {
    await ai.load();
    enabled = ai.getValue<boolean>("ai.enabled") ?? false;
    provider = ai.getValue<string>("ai.provider") ?? "ollama-default";
    providerAtLoad = provider;
    await refreshStatus();
  });

  async function setEnabled(value: boolean): Promise<void> {
    enabled = value;
    await ai.setValue("ai.enabled", value);
    // The daemon's ai.toml watcher applies this; re-probe shortly so
    // the status group reflects the new state.
    setTimeout(refreshStatus, 400);
  }

  async function setProvider(value: string): Promise<void> {
    provider = value;
    await ai.setValue("ai.provider", value);
  }

  const providerRestartPending = $derived(provider !== providerAtLoad);
</script>

<SettingsPage
  title="AI"
  description="On-device and cloud AI features. Off by default, so you stay in control of what the assistant can read."
>
  <Group label="AI Layer">
    <Row
      label="Enable AI features"
      description="Lets the assistant answer questions about your files and projects. Nothing runs until you turn this on."
      id="ai-enable"
    >
      {#snippet control()}
        <Switch
          value={enabled}
          ariaLabel="Enable AI features"
          onchange={setEnabled}
        />
      {/snippet}
    </Row>
  </Group>

  <Group label="Daemon Status">
    {#if statusError}
      <div class="error-box">
        <AlertCircle size={14} strokeWidth={2} />
        <span>{statusError}</span>
      </div>
    {:else}
      <Row
        label="AI Daemon"
        description="Answers queries and runs the Cypher pipeline."
        id="ai-daemon-status"
      >
        {#snippet control()}
          <span class="meta-text" class:positive={status?.daemonRunning}>
            {status?.daemonRunning ? "Running ✓" : "Stopped"}
          </span>
        {/snippet}
      </Row>
      <Row
        label="Network Proxy"
        description="The only path AI traffic takes to leave this machine."
        id="ai-proxy-status"
      >
        {#snippet control()}
          <span class="meta-text" class:positive={status?.proxyRunning}>
            {status?.proxyRunning ? "Running ✓" : "Stopped"}
          </span>
        {/snippet}
      </Row>
    {/if}

    <div class="footer-row">
      <Button
        variant="ghost"
        size="sm"
        disabled={statusLoading}
        onclick={refreshStatus}
      >
        <RefreshCw size={14} class={statusLoading ? "spin" : ""} />
        Refresh
      </Button>
    </div>
  </Group>

  <Group label="Provider">
    <Row
      label="Model provider"
      description="Which model answers your queries. Ollama runs entirely on this machine."
      id="ai-provider"
    >
      {#snippet control()}
        <PopoverSelect
          value={provider}
          options={PROVIDERS}
          ariaLabel="AI model provider"
          width="180px"
          onchange={setProvider}
        />
      {/snippet}
    </Row>
    {#if providerRestartPending}
      <Row
        label="Restart needed"
        description="The provider change applies after the AI daemon restarts."
        id="ai-provider-restart"
      >
        {#snippet control()}
          <span class="meta-text">
            <Sparkles size={12} strokeWidth={1.5} />
            pending
          </span>
        {/snippet}
      </Row>
    {/if}
  </Group>
</SettingsPage>

<style>
  .meta-text {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }

  .meta-text.positive {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }

  .error-box {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0 1rem 0.625rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    border-radius: var(--radius-chip);
    font-size: 0.8125rem;
    color: var(--foreground);
  }

  .error-box :global(svg) {
    flex-shrink: 0;
    color: var(--destructive);
    margin-top: 1px;
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
