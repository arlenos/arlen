<script lang="ts">
  /// AI settings page — configures `~/.config/lunaris/ai.toml`.
  ///
  /// Built on the design-system canon (docs/architecture/settings-app.md §0.3):
  /// Page/SectionGrid/Group/Row/Switch/SegmentedControl/ChipList from
  /// `@lunaris/ui-kit`; Button/Input/NumberInput/PopoverSelect are app-local
  /// (Tailwind/lucide) until the @source consolidation (S-U1b).
  ///
  /// Sections built here are the confirmed config keys + daemon status.
  /// External-content screening (the `[classifier]` schema), the behaviours
  /// list (needs a SKILL.md discovery command), and the Activity timeline (needs
  /// the audit-ledger read command) are sub-steps S-U3b/S-U4, not yet wired.

  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { Sparkles, RefreshCw, AlertCircle } from "lucide-svelte";
  import { Page } from "@lunaris/ui-kit/components/ui/page";
  import { SectionGrid } from "@lunaris/ui-kit/components/ui/section-grid";
  import { Group } from "@lunaris/ui-kit/components/ui/group";
  import { Row } from "@lunaris/ui-kit/components/ui/row";
  import { Switch } from "@lunaris/ui-kit/components/ui/switch";
  import { SegmentedControl } from "@lunaris/ui-kit/components/ui/segmented-control";
  import { ChipList } from "@lunaris/ui-kit/components/ui/chip-list";
  import { Button } from "@lunaris/ui-kit/components/ui/button";
  import { Input } from "@lunaris/ui-kit/components/ui/input";
  import { NumberInput } from "@lunaris/ui-kit/components/ui/number-input";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import { ai } from "$lib/stores/ai";

  interface AiStatus {
    daemonRunning: boolean;
    proxyRunning: boolean;
  }

  const PROVIDERS = [{ value: "ollama-default", label: "Ollama (local)" }];

  const ACCESS_LEVELS = [
    { value: "0", label: "Minimal" },
    { value: "1", label: "Session" },
    { value: "2", label: "Project" },
    { value: "3", label: "Time" },
    { value: "4", label: "Full" },
  ];
  const ACCESS_HINTS: Record<string, string> = {
    "0": "The assistant sees almost nothing of your graph.",
    "1": "Limited to the current session's activity.",
    "2": "The active project's files and context.",
    "3": "A recent time window across projects.",
    "4": "The whole Knowledge Graph.",
  };
  const ACTION_MODES = [
    { value: "suggest", label: "Suggest" },
    { value: "supervised", label: "Supervised" },
  ];

  let status = $state<AiStatus | null>(null);
  let statusLoading = $state(false);
  let statusError = $state<string | null>(null);

  let enabled = $state(false);
  let provider = $state("ollama-default");
  let providerAtLoad = $state("ollama-default");
  let model = $state("");
  let accessLevel = $state("0");
  let actionMode = $state("suggest");
  let autonomousApps = $state<string[]>([]);
  let executorLive = $state(false);
  let contextWindow = $state(8192);

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
    accessLevel = String(ai.getValue<number>("ai.access_level") ?? 0);
    actionMode = ai.getValue<string>("ai.action_mode") ?? "suggest";
    autonomousApps = ai.getValue<string[]>("ai.autonomous_apps") ?? [];
    executorLive = ai.getValue<boolean>("agent.executor_live") ?? false;
    model = ai.getValue<string>("provider.model") ?? "";
    contextWindow = ai.getValue<number>("provider.context_window") ?? 8192;
    await refreshStatus();
  });

  async function setEnabled(v: boolean) {
    enabled = v;
    await ai.setValue("ai.enabled", v);
    setTimeout(refreshStatus, 400);
  }
  async function setProvider(v: string) {
    provider = v;
    await ai.setValue("ai.provider", v);
  }
  async function setAccessLevel(v: string) {
    accessLevel = v;
    await ai.setValue("ai.access_level", Number(v));
  }
  async function setActionMode(v: string) {
    actionMode = v;
    await ai.setValue("ai.action_mode", v);
  }
  async function persistAutonomousApps() {
    await ai.setValue("ai.autonomous_apps", autonomousApps);
  }
  async function setExecutorLive(v: boolean) {
    executorLive = v;
    await ai.setValue("agent.executor_live", v);
  }
  async function setModel(v: string) {
    model = v;
    await ai.setValue("provider.model", v);
  }
  async function setContextWindow(v: number) {
    contextWindow = v;
    await ai.setValue("provider.context_window", v);
  }

  const providerRestartPending = $derived(provider !== providerAtLoad);
</script>

<Page
  title="AI"
  description="On-device and cloud AI features. Off by default, so you stay in control of what the assistant can read and do."
>
  <SectionGrid>
    <Group label="AI Layer">
      <Row
        label="Enable AI features"
        description="Lets the assistant answer questions and run behaviours. Nothing runs until you turn this on."
        id="ai-enable"
      >
        {#snippet control()}
          <Switch value={enabled} ariaLabel="Enable AI features" onchange={setEnabled} />
        {/snippet}
      </Row>
    </Group>

    <Group label="Provider">
      <Row label="Model provider" description="Ollama runs entirely on this machine." id="ai-provider">
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
      <Row label="Model" description="Model identifier the provider serves (blank uses the default)." id="ai-model">
        {#snippet control()}
          <Input value={model} placeholder="llama3:8b" oninput={(e) => setModel(e.currentTarget.value)} />
        {/snippet}
      </Row>
      <Row label="Context window" description="Usable input tokens; the loop compacts to fit." id="ai-context-window">
        {#snippet control()}
          <NumberInput value={contextWindow} min={2048} max={131072} step={1024} unit="tok" onchange={setContextWindow} />
        {/snippet}
      </Row>
      {#if providerRestartPending}
        <Row label="Restart needed" description="The provider change applies after the AI daemon restarts." id="ai-provider-restart">
          {#snippet control()}
            <span class="meta"><Sparkles size={12} strokeWidth={1.5} />pending</span>
          {/snippet}
        </Row>
      {/if}
    </Group>

    <Group label="Access">
      <Row
        label="Knowledge read level"
        description={ACCESS_HINTS[accessLevel] ?? ""}
        id="ai-access-level"
      >
        {#snippet below()}
          <SegmentedControl
            value={accessLevel}
            options={ACCESS_LEVELS}
            ariaLabel="Knowledge read level"
            onchange={setAccessLevel}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label="Actions">
      <Row label="Action mode" description="How the assistant is allowed to act." id="ai-action-mode">
        {#snippet below()}
          <SegmentedControl
            value={actionMode}
            options={ACTION_MODES}
            ariaLabel="Action mode"
            onchange={setActionMode}
          />
        {/snippet}
      </Row>
      <Row
        label="Always-confirm rule"
        description="High-impact actions (delete, send, install) and anything triggered by external content always ask first, regardless of mode."
        id="ai-confirm-rule"
      >
        {#snippet control()}
          <span class="meta">enforced</span>
        {/snippet}
      </Row>
      <Row
        label="Autonomous apps"
        description={autonomousApps.length === 0
          ? "No app may act autonomously. Add an app id to allow it (per-app only; never global)."
          : "These apps may act without confirmation in their own scope."}
        id="ai-autonomous-apps"
      >
        {#snippet below()}
          <ChipList
            bind:items={autonomousApps}
            placeholder="Add an app id, e.g. org.lunaris.files"
            onchange={persistAutonomousApps}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label="Execution">
      <Row
        label="Allow safe curation to write"
        description="Lets deterministic curation workflows (e.g. auto-tagging files to projects) write the graph without a per-action prompt. Review results in Activity. Off by default; the write still passes the full gate."
        id="ai-executor-live"
      >
        {#snippet control()}
          <Switch value={executorLive} ariaLabel="Allow safe curation to write" onchange={setExecutorLive} />
        {/snippet}
      </Row>
    </Group>

    <Group label="Status">
      {#if statusError}
        <Row label="Status unavailable" description={statusError} id="ai-status-error">
          {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
        </Row>
      {:else}
        <Row label="AI Daemon" description="Answers queries and runs the Cypher pipeline." id="ai-daemon-status">
          {#snippet control()}
            <span class="meta" class:on={status?.daemonRunning}>{status?.daemonRunning ? "Running" : "Stopped"}</span>
          {/snippet}
        </Row>
        <Row label="Network Proxy" description="The only path AI traffic takes to leave this machine." id="ai-proxy-status">
          {#snippet control()}
            <span class="meta" class:on={status?.proxyRunning}>{status?.proxyRunning ? "Running" : "Stopped"}</span>
          {/snippet}
        </Row>
      {/if}
      <Row label="Refresh" description="Re-probe the daemon and proxy." id="ai-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={statusLoading} onclick={refreshStatus}>
            <RefreshCw size={14} class={statusLoading ? "ai-spin" : ""} />
            Refresh
          </Button>
        {/snippet}
      </Row>
    </Group>
  </SectionGrid>
</Page>

<style>
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .meta.on {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  :global(.ai-spin) {
    animation: ai-spin 0.8s linear infinite;
  }
  @keyframes ai-spin {
    to {
      transform: rotate(360deg);
    }
  }
  :global(.ai-error-icon) {
    color: var(--destructive);
  }
</style>
