<script lang="ts">
  /// AI settings, the trust + control home for the AI layer: enable it, set how
  /// freely it acts (the decided action_mode + executor_live model), how much it
  /// can see, what it does on its own, plus links out to the Providers and
  /// Default-models views and a health line. Configures `~/.config/arlen/ai.toml`.
  ///
  /// Reviewing what the AI did lives in the AI app's Activity feed, not here
  /// (one activity home, one config home). The behaviours list is wired against
  /// `ai_behaviours` (read) + `ai_behaviour_set_enabled` (write); until the
  /// backend provides the write the rows report that honestly. Choosing the
  /// provider + model lives in the linked views, not here.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { Sparkles, RefreshCw, AlertCircle, Cloud, SlidersHorizontal } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ChoiceList } from "@arlen/ui-kit/components/ui/choice-list";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { Button } from "@arlen/ui-kit/components/ui/button";
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

  // The read level (the daemon stores 0-4 in `ai.access_level`). This gates how
  // much of your ACTIVITY the assistant draws on as context, not file access
  // (reading a file's contents is a separate path you trigger with @). Each row
  // says plainly how much context it uses.
  const ACCESS_CHOICES = [
    {
      value: "0",
      label: "Just this chat",
      description: "It uses only what you bring up in the conversation, none of your activity.",
    },
    {
      value: "1",
      label: "This session",
      description: "What you are working on right now, in this session.",
    },
    {
      value: "2",
      label: "This project",
      description: "The project you are focused on, so it understands what you are working on.",
      note: "Follows the project you focus in the shell; with none focused it uses nothing here.",
    },
    {
      value: "3",
      label: "Your recent work",
      description: "What you have worked on over the last few days, so it has useful context.",
    },
    {
      value: "4",
      label: "Everything",
      description: "All of your activity and history.",
    },
  ];
  // How freely it acts, the baseline posture. "supervised" only takes effect
  // once the executor master below is on.
  const ACTION_MODES = [
    { value: "suggest", label: "Suggests only" },
    { value: "supervised", label: "Acts with a preview" },
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
  let accessLevel = $state("0");
  let actionMode = $state("suggest");
  let autonomousApps = $state<string[]>([]);
  let executorLive = $state(false);

  // Live info for the two link cards, loaded from the daemon (IPC-caveat: keep
  // the async-filled data in writable stores). Honest fallbacks when the daemon
  // is unreachable.
  const providerLine = writable("Connect a local or cloud service");
  const defaultModelLine = writable("No model chosen yet");

  function parse<T>(json: string, fallback: T): T {
    try {
      return JSON.parse(json) as T;
    } catch {
      return fallback;
    }
  }

  interface ProviderRow {
    name: string;
    enabled: boolean;
    configured: boolean;
  }

  async function loadCards() {
    try {
      const list = parse<ProviderRow[]>(await invoke<string>("ai_providers_list"), []);
      if (list.length > 0) {
        const connected = list.filter((p) => p.enabled && p.configured).length;
        const names = list.map((p) => p.name).slice(0, 2).join(", ");
        providerLine.set(
          connected > 0
            ? `${names}${list.length > 2 ? " and more" : ""} · ${connected} connected`
            : `${names}${list.length > 2 ? " and more" : ""} · none connected`,
        );
      }
    } catch {
      // keep the fallback line
    }
    try {
      const def = parse<{ provider?: string; model?: string }>(
        await invoke<string>("ai_defaults_get"),
        {},
      );
      if (def.model) {
        defaultModelLine.set(def.provider ? `${def.provider} · ${def.model}` : def.model);
      }
    } catch {
      // keep the fallback line
    }
  }

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
    accessLevel = String(ai.getValue<number>("ai.access_level") ?? 0);
    actionMode = ai.getValue<string>("ai.action_mode") ?? "suggest";
    autonomousApps = ai.getValue<string[]>("ai.autonomous_apps") ?? [];
    executorLive = ai.getValue<boolean>("agent.executor_live") ?? false;
    await refreshStatus();
    await loadBehaviours();
    await loadCards();
  });

  async function setEnabled(v: boolean) {
    enabled = v;
    await ai.setValue("ai.enabled", v);
    setTimeout(refreshStatus, 400);
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

  // The honest hint under the action-mode control: a supervised baseline does
  // nothing until the executor master is on.
  const actsHint = $derived(
    actionMode === "supervised" && !executorLive
      ? "It still only suggests until you let it act, below."
      : actionMode === "supervised"
        ? "It carries out small, reversible things after showing you a preview you can cancel."
        : "It proposes each action and you run it yourself.",
  );
</script>

<Page
  title="General"
  description="Your assistant: how much it can see and do. Off by default, so you stay in control. What it has done shows in the AI app."
>
  <SectionGrid>
    <Group label="Assistant">
      <Row
        label="Enable the assistant"
        description="Lets it answer questions and work in the background. Nothing runs until you turn this on."
        id="ai-enable"
      >
        {#snippet control()}
          <Switch value={enabled} ariaLabel="Enable the assistant" onchange={setEnabled} />
        {/snippet}
      </Row>
    </Group>

    <Group label="How freely it acts">
      <Row label="When it acts" description={actsHint} id="ai-action-mode">
        {#snippet below()}
          <SegmentedControl
            value={actionMode}
            options={ACTION_MODES}
            ariaLabel="How freely the assistant acts"
            onchange={setActionMode}
          />
        {/snippet}
      </Row>
      <Row
        label="Let it act on its own"
        description="The master switch for acting. Until this is on, it only suggests, whatever you pick above. Every change it makes is reversible and shows in the AI app."
        id="ai-executor-live"
      >
        {#snippet control()}
          <Switch value={executorLive} ariaLabel="Let it act on its own" onchange={setExecutorLive} />
        {/snippet}
      </Row>
      <Row
        label="Always-confirm rule"
        description="High-impact actions (delete, send, install) and anything triggered by outside content always ask first, whatever the setting."
        id="ai-confirm-rule"
      >
        {#snippet control()}
          <span class="meta">enforced</span>
        {/snippet}
      </Row>
      <Row
        label="Per-app exceptions"
        description={autonomousApps.length === 0
          ? "No app may act on its own. Add an app id to let that one app act without asking, in its own scope only."
          : "These apps may act without asking, each in its own scope."}
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

    <Group label="What it can see">
      <Row
        label="How much it draws on"
        description="How much of your activity the assistant uses as context, so it can actually help. It uses your recent work by default; narrow this if you prefer. Every read is logged, and you can turn it down anytime."
        id="ai-access-level"
      >
        {#snippet below()}
          <ChoiceList
            value={accessLevel}
            options={ACCESS_CHOICES}
            ariaLabel="What the assistant may read"
            onchange={setAccessLevel}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label="What it does on its own">
      <Row
        label="Background tasks"
        description="Each is one background task. Turn them on or off here; everything they do shows in the AI app."
        id="ai-behaviours"
      ></Row>
      {#if behavioursUnavailable}
        <Row label="Tasks unavailable" description="Can't read the task list right now." id="ai-behaviours-unavailable">
          {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
        </Row>
      {:else if behaviours}
        {#if behaviours.behaviours.length === 0}
          <Row
            label="No background tasks"
            description="When apps or the system add background tasks, they appear here."
            id="ai-behaviours-empty"
          ></Row>
        {/if}
        {#each behaviours.behaviours as b (b.name)}
          <Row
            label={b.name}
            description={behaviourWriteFailed[b.name] ? "Could not save this change." : b.description}
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
            label="Some tasks could not be read"
            description={behaviours.errors.join("; ")}
            id="ai-behaviours-errors"
          >
            {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
          </Row>
        {/if}
      {/if}
    </Group>

    <Group label="Where AI comes from">
      <LinkCard href="/ai/providers" title="Providers" description={$providerLine}>
        {#snippet icon()}<Cloud size={20} strokeWidth={1.75} />{/snippet}
      </LinkCard>
      <LinkCard href="/ai/models" title="Default models" description={$defaultModelLine}>
        {#snippet icon()}<SlidersHorizontal size={20} strokeWidth={1.75} />{/snippet}
      </LinkCard>
    </Group>

    <Group label="Health">
      {#if statusError}
        <Row label="Status unavailable" description="Can't check the services right now." id="ai-status-error">
          {#snippet control()}
            <span title={statusError}><AlertCircle size={16} class="ai-error-icon" /></span>
          {/snippet}
        </Row>
      {:else}
        <Row
          label="Services"
          description="The assistant answers in the AI app; the network gate is the only path AI traffic can take off this machine."
          id="ai-services"
        >
          {#snippet control()}
            <span class="health">
              <span class="meta" class:on={status?.daemonRunning}>
                {status?.daemonRunning ? "Assistant on" : "Assistant off"}
              </span>
              <span class="health-sep">·</span>
              <span class="meta" class:on={status?.proxyRunning}>
                {status?.proxyRunning ? "Gate on" : "Gate off"}
              </span>
            </span>
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
        description="A plain summary of what your computer is doing right now. Needs the Everything read level."
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
  .health {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .health-sep {
    color: color-mix(in srgb, var(--foreground) 30%, transparent);
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
