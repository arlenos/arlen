<script lang="ts">
  /// AI settings page, the single CONFIG home for the AI layer (settings-app.md
  /// §0.3; the chat/agent split resolution in harness-redo-plan.md): master
  /// switch, provider, read level, action mode, behaviours, execution, and
  /// service status. Configures `~/.config/arlen/ai.toml`.
  ///
  /// Reviewing what the AI did lives in the AI app's Activity feed, not here
  /// (one activity home, one config home). The behaviours list is wired
  /// against the intended commands (`ai_behaviours` read,
  /// `ai_behaviour_set_enabled` write); until the backend provides them the
  /// rows report that honestly.

  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Sparkles, RefreshCw, AlertCircle } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ai } from "$lib/stores/ai";

  interface AiStatus {
    daemonRunning: boolean;
    proxyRunning: boolean;
  }

  /// One background behaviour as the registry reports it.
  interface BehaviourStatus {
    name: string;
    description: string;
    kind: string;
    provenance: string;
    enabled: boolean;
    disabledReason: string | null;
    reads: string;
  }
  interface BehaviourReport {
    behaviours: BehaviourStatus[];
    errors: string[];
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
    "0": "The assistant sees almost nothing.",
    "1": "Limited to the current session's activity.",
    "2": "The active project's files and context.",
    "3": "A recent time window across projects.",
    "4": "Everything the system has recorded about your files and activity.",
  };
  const ACTION_MODES = [
    { value: "suggest", label: "Suggest" },
    { value: "supervised", label: "Supervised" },
  ];

  let status = $state<AiStatus | null>(null);
  let statusLoading = $state(false);
  let statusError = $state<string | null>(null);

  // System Explanation Mode (Foundation §5.8): an on-demand plain-language
  // summary of what the computer is doing now.
  let explanation = $state<string | null>(null);
  let explainError = $state<string | null>(null);
  let explaining = $state(false);

  async function runExplain() {
    explaining = true;
    explainError = null;
    try {
      explanation = await invoke<string>("ai_explain");
    } catch (e) {
      explainError = String(e);
      explanation = null;
    } finally {
      explaining = false;
    }
  }

  let enabled = $state(false);
  let provider = $state("ollama-default");
  let providerAtLoad = $state("ollama-default");
  let model = $state("");
  let accessLevel = $state("0");
  let actionMode = $state("suggest");
  let autonomousApps = $state<string[]>([]);
  let executorLive = $state(false);
  let contextWindow = $state(8192);

  // The behaviours read. `null` before the first read settles; `unavailable`
  // when the command failed (it does not exist in this app's backend yet).
  let behaviours = $state<BehaviourReport | null>(null);
  let behavioursUnavailable = $state(false);
  // Behaviour names whose toggle write failed, shown honestly per row.
  let behaviourWriteFailed = $state<Record<string, boolean>>({});

  async function loadBehaviours(): Promise<void> {
    try {
      behaviours = await invoke<BehaviourReport>("ai_behaviours");
      behavioursUnavailable = false;
    } catch {
      behaviours = null;
      behavioursUnavailable = true;
    }
  }

  async function setBehaviourEnabled(name: string, v: boolean): Promise<void> {
    // Optimistic flip; on failure flip back and say so on the row.
    behaviours = behaviours && {
      ...behaviours,
      behaviours: behaviours.behaviours.map((b) => (b.name === name ? { ...b, enabled: v } : b)),
    };
    try {
      await invoke("ai_behaviour_set_enabled", { name, enabled: v });
      behaviourWriteFailed = { ...behaviourWriteFailed, [name]: false };
    } catch {
      behaviours = behaviours && {
        ...behaviours,
        behaviours: behaviours.behaviours.map((b) =>
          b.name === name ? { ...b, enabled: !v } : b,
        ),
      };
      behaviourWriteFailed = { ...behaviourWriteFailed, [name]: true };
    }
  }

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
    await loadBehaviours();
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
  description="On-device and cloud AI features. Off by default, so you stay in control of what the assistant can read and do. What it has done shows in the AI app."
>
  <SectionGrid>
    <Group label="AI Layer">
      <Row
        label="Enable AI features"
        description="Lets the assistant answer questions and work in the background. Nothing runs until you turn this on."
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
            onchange={setProvider}
          />
        {/snippet}
      </Row>
      <Row label="Model" description="Model identifier the provider serves (blank uses the default)." id="ai-model">
        {#snippet control()}
          <Input class="row-control" value={model} placeholder="llama3:8b" oninput={(e) => setModel(e.currentTarget.value)} />
        {/snippet}
      </Row>
      <Row label="Context window" description="How much text the model can take in at once, in tokens." id="ai-context-window">
        {#snippet control()}
          <NumberInput width="var(--width-row-control, 200px)" value={contextWindow} min={2048} max={131072} step={1024} unit="tok" onchange={setContextWindow} />
        {/snippet}
      </Row>
      {#if providerRestartPending}
        <Row label="Restart needed" description="The provider change applies after the assistant restarts." id="ai-provider-restart">
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
        description="High-impact actions (delete, send, install) and anything triggered by outside content always ask first, regardless of mode."
        id="ai-confirm-rule"
      >
        {#snippet control()}
          <span class="meta">enforced</span>
        {/snippet}
      </Row>
      <Row
        label="Autonomous apps"
        description={autonomousApps.length === 0
          ? "No app may act on its own. Add an app id to allow it (per app only, never global)."
          : "These apps may act without confirmation in their own scope."}
        id="ai-autonomous-apps"
      >
        {#snippet below()}
          <ChipList
            bind:items={autonomousApps}
            placeholder="Add an app id, e.g. org.arlen.files"
            onchange={persistAutonomousApps}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label="Behaviours">
      <Row
        label="What it may do on its own"
        description="Each behaviour is one background task. Turn them on or off here; everything they do shows in the AI app."
        id="ai-behaviours"
      ></Row>
      {#if behavioursUnavailable}
        <Row
          label="Behaviour list unavailable"
          description="Can't read the behaviour list right now."
          id="ai-behaviours-unavailable"
        >
          {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
        </Row>
      {:else if behaviours}
        {#if behaviours.behaviours.length === 0}
          <Row
            label="No behaviours installed"
            description="When apps or the system add background tasks, they appear here."
            id="ai-behaviours-empty"
          ></Row>
        {/if}
        {#each behaviours.behaviours as b (b.name)}
          <Row
            label={b.name}
            description={behaviourWriteFailed[b.name]
              ? "Could not save this change."
              : b.description}
            id={`ai-behaviour-${b.name}`}
          >
            {#snippet control()}
              <Switch
                value={b.enabled}
                ariaLabel={`Enable ${b.name}`}
                onchange={(v: boolean) => setBehaviourEnabled(b.name, v)}
              />
            {/snippet}
          </Row>
        {/each}
        {#if behaviours.errors.length > 0}
          <Row
            label="Some behaviours could not be read"
            description={behaviours.errors.join("; ")}
            id="ai-behaviours-errors"
          >
            {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
          </Row>
        {/if}
      {/if}
    </Group>

    <Group label="Execution">
      <Row
        label="Let it make small changes"
        description="Small, reversible changes like sorting files into projects, made without asking each time. Every change shows in the AI app and still passes the safety checks. Off by default."
        id="ai-executor-live"
      >
        {#snippet control()}
          <Switch value={executorLive} ariaLabel="Let it make small changes" onchange={setExecutorLive} />
        {/snippet}
      </Row>
    </Group>

    <Group label="Status">
      {#if statusError}
        <Row
          label="Status unavailable"
          description="Can't check the services right now."
          id="ai-status-error"
        >
          {#snippet control()}
            <span title={statusError}><AlertCircle size={16} class="ai-error-icon" /></span>
          {/snippet}
        </Row>
      {:else}
        <Row label="Assistant service" description="Answers your questions in the AI app." id="ai-daemon-status">
          {#snippet control()}
            <span class="meta" class:on={status?.daemonRunning}>{status?.daemonRunning ? "Running" : "Stopped"}</span>
          {/snippet}
        </Row>
        <Row label="Network gate" description="The only path AI traffic can take to leave this machine." id="ai-proxy-status">
          {#snippet control()}
            <span class="meta" class:on={status?.proxyRunning}>{status?.proxyRunning ? "Running" : "Stopped"}</span>
          {/snippet}
        </Row>
      {/if}
      <Row label="Refresh" description="Check the services again." id="ai-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={statusLoading} onclick={refreshStatus}>
            <RefreshCw size={14} class={statusLoading ? "ai-spin" : ""} />
            Refresh
          </Button>
        {/snippet}
      </Row>
    </Group>

    <Group label="What's happening now">
      <Row
        label="Explain my system"
        description="A plain summary of what your computer is doing right now. Needs the Full read level."
        id="ai-explain"
      >
        {#snippet control()}
          <Button variant="outline" size="sm" disabled={explaining} onclick={runExplain}>
            <Sparkles size={14} class={explaining ? "ai-spin" : ""} />
            {explaining ? "Working" : "Explain"}
          </Button>
        {/snippet}
        {#snippet below()}
          {#if explainError}
            <p class="explain-error" title={explainError}>Could not build an explanation. Try again.</p>
          {:else if explanation}
            <p class="explain-text">{explanation}</p>
          {/if}
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
  .explain-text {
    margin: 0.5rem 0 0;
    font-size: 0.875rem;
    line-height: 1.55;
    color: var(--foreground);
    white-space: pre-wrap;
  }
  .explain-error {
    margin: 0.5rem 0 0;
    font-size: 0.8125rem;
    color: var(--color-error);
  }
</style>
