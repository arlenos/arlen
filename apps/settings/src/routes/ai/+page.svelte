<script lang="ts">
  /// AI settings page — configures `~/.config/lunaris/ai.toml`.
  ///
  /// Built on the design-system canon (docs/architecture/settings-app.md §0.3):
  /// Page/SectionGrid/Group/Row/Switch/SegmentedControl/ChipList from
  /// `@lunaris/ui-kit`; Button/Input/NumberInput/PopoverSelect are app-local
  /// (Tailwind/lucide) until the @source consolidation (S-U1b).
  ///
  /// Sections built here are the confirmed config keys, daemon status, and the
  /// read-only Activity timeline (S-U4, the audit-ledger read command). The
  /// External-content screening (`[classifier]` schema) and the behaviours list
  /// (needs a SKILL.md discovery command) remain sub-steps S-U3b, not yet wired.

  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { Sparkles, RefreshCw, AlertCircle, ShieldAlert, History } from "lucide-svelte";
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
  import { PopoverSelect } from "@lunaris/ui-kit/components/ui/popover-select";
  import { ai } from "$lib/stores/ai";

  interface AiStatus {
    daemonRunning: boolean;
    proxyRunning: boolean;
  }

  interface ActivityEntry {
    index: number;
    timestampMicros: number;
    kind: string;
    actor: string;
    subject: string;
    outcome: string;
    nodeTypes: string[];
    relations: string[];
    resultCount: number | null;
    durationMs: number | null;
    depth: number | null;
    callChainId: string | null;
    projectId: string | null;
    entryRef: string;
  }

  interface ActivityPage {
    entries: ActivityEntry[];
    available: boolean;
    tampered: boolean;
    total: number;
  }

  /// Human label per audit kind, paired with a semantic tone for the badge.
  const KIND_META: Record<string, { label: string; tone: string }> = {
    query: { label: "Query", tone: "neutral" },
    "tool-call": { label: "Tool call", tone: "info" },
    confirm: { label: "Confirmed", tone: "ok" },
    "policy-violation": { label: "Blocked", tone: "warn" },
    "graph-access": { label: "Graph", tone: "neutral" },
    permission: { label: "Permission", tone: "info" },
    "network-call": { label: "Network", tone: "info" },
  };

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

  let activity = $state<ActivityPage | null>(null);
  let activityLoading = $state(false);
  let activityError = $state<string | null>(null);

  /// A coarse relative-time label from a microsecond Unix timestamp.
  function relativeTime(micros: number): string {
    const then = micros / 1000;
    const diffSec = Math.max(0, (Date.now() - then) / 1000);
    if (diffSec < 45) return "just now";
    if (diffSec < 90) return "a minute ago";
    const min = Math.round(diffSec / 60);
    if (min < 60) return `${min} min ago`;
    const hr = Math.round(min / 60);
    if (hr < 24) return `${hr} h ago`;
    const day = Math.round(hr / 24);
    if (day < 7) return `${day} d ago`;
    return new Date(then).toLocaleDateString();
  }

  async function loadActivity(): Promise<void> {
    activityLoading = true;
    activityError = null;
    try {
      activity = await invoke<ActivityPage>("ai_activity_recent", { limit: 50 });
    } catch (e) {
      activityError = String(e);
      activity = null;
    } finally {
      activityLoading = false;
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
    await loadActivity();
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

    <Group label="Activity" class="span-full">
      <div class="activity-head" id="ai-activity">
        <div class="activity-head-text">
          <History size={15} strokeWidth={1.75} />
          <span>
            What the assistant has done. Read-only, from the tamper-evident audit
            ledger.{#if activity && activity.total > 0}
              <span class="activity-count">{activity.total} total</span>{/if}
          </span>
        </div>
        <Button variant="ghost" size="sm" disabled={activityLoading} onclick={loadActivity}>
          <RefreshCw size={14} class={activityLoading ? "ai-spin" : ""} />
          Refresh
        </Button>
      </div>

      {#if activity?.tampered}
        <div class="activity-banner">
          <ShieldAlert size={16} />
          The audit ledger reports tampering. The entries below may be incomplete.
        </div>
      {/if}

      {#if activityError}
        <p class="activity-empty">Activity unavailable: {activityError}</p>
      {:else if !activity || !activity.available}
        <p class="activity-empty">
          The audit daemon is not running, so there is no activity to show yet.
        </p>
      {:else if activity.entries.length === 0}
        <p class="activity-empty">No AI activity recorded yet.</p>
      {:else}
        <ul class="activity-list">
          {#each activity.entries as entry (entry.entryRef)}
            {@const meta = KIND_META[entry.kind] ?? { label: entry.kind, tone: "neutral" }}
            <li class="activity-item">
              <span class="activity-badge" data-tone={meta.tone}>{meta.label}</span>
              <div class="activity-body">
                <div class="activity-line">
                  <span class="activity-subject">{entry.subject}</span>
                  <span class="activity-outcome" data-outcome={entry.outcome}>{entry.outcome}</span>
                </div>
                <div class="activity-detail">
                  <span>{entry.actor}</span>
                  {#if entry.relations.length > 0}
                    <span class="activity-sep">·</span>
                    <span>{entry.relations.join(", ")}</span>
                  {/if}
                  {#if entry.resultCount !== null}
                    <span class="activity-sep">·</span>
                    <span>{entry.resultCount} result{entry.resultCount === 1 ? "" : "s"}</span>
                  {/if}
                  {#if entry.durationMs !== null}
                    <span class="activity-sep">·</span>
                    <span>{entry.durationMs} ms</span>
                  {/if}
                </div>
              </div>
              <time class="activity-time">{relativeTime(entry.timestampMicros)}</time>
            </li>
          {/each}
        </ul>
      {/if}
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

  .activity-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.25rem;
  }
  .activity-head-text {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    line-height: 1.4;
  }
  .activity-head-text :global(svg) {
    flex-shrink: 0;
    margin-top: 0.1rem;
  }
  .activity-count {
    margin-left: 0.375rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .activity-banner {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 0.75rem;
    margin-bottom: 0.75rem;
    border-radius: 0.5rem;
    font-size: 0.8125rem;
    color: var(--destructive);
    background: color-mix(in srgb, var(--destructive) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--destructive) 30%, transparent);
  }
  .activity-empty {
    margin: 0;
    padding: 1rem 0;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .activity-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .activity-item {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.625rem 0;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .activity-item:first-child {
    border-top: none;
  }
  .activity-badge {
    flex-shrink: 0;
    min-width: 5.5rem;
    text-align: center;
    padding: 0.125rem 0.5rem;
    border-radius: 0.375rem;
    font-size: 0.6875rem;
    font-weight: 500;
    letter-spacing: 0.01em;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .activity-badge[data-tone="ok"] {
    color: #16a34a;
    background: color-mix(in srgb, #16a34a 14%, transparent);
  }
  .activity-badge[data-tone="warn"] {
    color: var(--destructive);
    background: color-mix(in srgb, var(--destructive) 14%, transparent);
  }
  .activity-badge[data-tone="info"] {
    color: var(--accent, #6366f1);
    background: color-mix(in srgb, var(--accent, #6366f1) 14%, transparent);
  }
  .activity-body {
    flex: 1;
    min-width: 0;
  }
  .activity-line {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }
  .activity-subject {
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .activity-outcome {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .activity-outcome[data-outcome="denied"],
  .activity-outcome[data-outcome="error"] {
    color: var(--destructive);
  }
  .activity-detail {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.125rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .activity-sep {
    opacity: 0.5;
  }
  .activity-time {
    flex-shrink: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
</style>
