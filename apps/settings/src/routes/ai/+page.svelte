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
  import { t } from "$lib/i18n/messages";
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
  // (reading a file's contents is a separate path you trigger with @). Settings
  // surfaces the three meaningful amounts on one axis (how much of your recent
  // activity); the niche session/project scopes (1/2) stay in the backend, not
  // here. Each row says plainly how much context it uses.
  const ACCESS_CHOICES = $derived([
    {
      value: "0",
      label: $t("s.ai.access.chat"),
      description: $t("s.ai.access.chat.desc"),
    },
    {
      value: "3",
      label: $t("s.ai.access.recent"),
      description: $t("s.ai.access.recent.desc"),
      note: $t("s.ai.access.recent.note"),
    },
    {
      value: "4",
      label: $t("s.ai.access.everything"),
      description: $t("s.ai.access.everything.desc"),
    },
  ]);
  // How freely it acts, the baseline posture. "supervised" only takes effect
  // once the executor master below is on.
  const ACTION_MODES = $derived([
    { value: "suggest", label: $t("s.ai.mode.suggest") },
    { value: "supervised", label: $t("s.ai.mode.supervised") },
  ]);

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
  // Generous default (TimeScoped, recent activity): the AI is useful out of
  // the box once enabled; the user narrows if they want. Matches the daemon
  // fallback + the shipped ai.toml.
  let accessLevel = $state("3");
  // The picker surfaces 0/3/4 only. If a stored value is a non-surfaced scope
  // (1/2, set via another path), show "recent work" selected without rewriting
  // it; the user's next pick persists a surfaced value.
  const displayAccessLevel = $derived(
    ["0", "3", "4"].includes(accessLevel) ? accessLevel : "3",
  );
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
    accessLevel = String(ai.getValue<number>("ai.access_level") ?? 3);
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
      ? $t("s.ai.actsHint.supervisedOff")
      : actionMode === "supervised"
        ? $t("s.ai.actsHint.supervised")
        : $t("s.ai.actsHint.suggest"),
  );
</script>

<Page
  title={$t("s.ai.title")}
  description={$t("s.ai.desc")}
>
  <SectionGrid>
    <Group label={$t("s.ai.assistant")}>
      <Row
        label={$t("s.ai.enable")}
        description={$t("s.ai.enable.desc")}
        id="ai-enable"
      >
        {#snippet control()}
          <Switch value={enabled} ariaLabel={$t("s.ai.enable")} onchange={setEnabled} />
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.ai.freely")}>
      <Row label={$t("s.ai.whenActs")} description={actsHint} id="ai-action-mode">
        {#snippet below()}
          <SegmentedControl
            value={actionMode}
            options={ACTION_MODES}
            ariaLabel={$t("s.ai.whenActs.aria")}
            onchange={setActionMode}
          />
        {/snippet}
      </Row>
      <Row
        label={$t("s.ai.onOwn")}
        description={$t("s.ai.onOwn.desc")}
        id="ai-executor-live"
      >
        {#snippet control()}
          <Switch value={executorLive} ariaLabel={$t("s.ai.onOwn")} onchange={setExecutorLive} />
        {/snippet}
      </Row>
      <Row
        label={$t("s.ai.confirmRule")}
        description={$t("s.ai.confirmRule.desc")}
        id="ai-confirm-rule"
      >
        {#snippet control()}
          <span class="meta">{$t("s.ai.enforced")}</span>
        {/snippet}
      </Row>
      <Row
        label={$t("s.ai.perApp")}
        description={autonomousApps.length === 0
          ? $t("s.ai.perApp.none")
          : $t("s.ai.perApp.some")}
        id="ai-autonomous-apps"
      >
        {#snippet below()}
          <ChipList
            bind:items={autonomousApps}
            placeholder={$t("s.ai.perApp.placeholder")}
            onchange={persistAutonomousApps}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.ai.canSee")}>
      <Row
        label={$t("s.ai.drawsOn")}
        description={$t("s.ai.drawsOn.desc")}
        id="ai-access-level"
      >
        {#snippet below()}
          <ChoiceList
            value={displayAccessLevel}
            options={ACCESS_CHOICES}
            ariaLabel={$t("s.ai.drawsOn.aria")}
            onchange={setAccessLevel}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.ai.doesOnOwn")}>
      <Row
        label={$t("s.ai.bgTasks")}
        description={$t("s.ai.bgTasks.desc")}
        id="ai-behaviours"
      ></Row>
      {#if behavioursUnavailable}
        <Row label={$t("s.ai.tasksUnavailable")} description={$t("s.ai.tasksUnavailable.desc")} id="ai-behaviours-unavailable">
          {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
        </Row>
      {:else if behaviours}
        {#if behaviours.behaviours.length === 0}
          <Row
            label={$t("s.ai.noTasks")}
            description={$t("s.ai.noTasks.desc")}
            id="ai-behaviours-empty"
          ></Row>
        {/if}
        {#each behaviours.behaviours as b (b.name)}
          <Row
            label={b.name}
            description={behaviourWriteFailed[b.name] ? $t("s.ai.taskSaveFailed") : b.description}
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
            label={$t("s.ai.tasksReadFailed")}
            description={behaviours.errors.join("; ")}
            id="ai-behaviours-errors"
          >
            {#snippet control()}<AlertCircle size={16} class="ai-error-icon" />{/snippet}
          </Row>
        {/if}
      {/if}
    </Group>

    <Group label={$t("s.ai.whereFrom")}>
      <LinkCard href="/ai/providers" title={$t("s.ai.providers")} description={$providerLine}>
        {#snippet icon()}<Cloud size={20} strokeWidth={1.75} />{/snippet}
      </LinkCard>
      <LinkCard href="/ai/models" title={$t("s.ai.models")} description={$defaultModelLine}>
        {#snippet icon()}<SlidersHorizontal size={20} strokeWidth={1.75} />{/snippet}
      </LinkCard>
    </Group>

    <Group label={$t("s.ai.health")}>
      {#if statusError}
        <Row label={$t("s.ai.statusUnavailable")} description={$t("s.ai.statusUnavailable.desc")} id="ai-status-error">
          {#snippet control()}
            <span title={statusError}><AlertCircle size={16} class="ai-error-icon" /></span>
          {/snippet}
        </Row>
      {:else}
        <Row
          label={$t("s.ai.services")}
          description={$t("s.ai.services.desc")}
          id="ai-services"
        >
          {#snippet control()}
            <span class="health">
              <span class="meta" class:on={status?.daemonRunning}>
                {status?.daemonRunning ? $t("s.ai.assistantOn") : $t("s.ai.assistantOff")}
              </span>
              <span class="health-sep">·</span>
              <span class="meta" class:on={status?.proxyRunning}>
                {status?.proxyRunning ? $t("s.ai.gateOn") : $t("s.ai.gateOff")}
              </span>
            </span>
          {/snippet}
        </Row>
      {/if}
      <Row label={$t("s.ai.refresh")} description={$t("s.ai.refresh.desc")} id="ai-refresh">
        {#snippet control()}
          <Button variant="ghost" size="sm" disabled={statusLoading} onclick={refreshStatus}>
            <RefreshCw size={14} class={statusLoading ? "ai-spin" : ""} />
            {$t("s.ai.refresh")}
          </Button>
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.ai.happeningNow")}>
      <Row
        label={$t("s.ai.explain")}
        description={$t("s.ai.explain.desc")}
        id="ai-explain"
      >
        {#snippet control()}
          <Button variant="outline" size="sm" disabled={explaining} onclick={runExplain}>
            <Sparkles size={14} class={explaining ? "ai-spin" : ""} />
            {explaining ? $t("s.ai.explain.working") : $t("s.ai.explain.action")}
          </Button>
        {/snippet}
        {#snippet below()}
          {#if explainError}
            <p class="explain-error" title={explainError}>{$t("s.ai.explain.failed")}</p>
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
