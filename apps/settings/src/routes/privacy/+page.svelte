<script lang="ts">
  /// App access - the system-wide capability browser (living-capability-graph.md
  /// §6). Shows every principal (the assistant and each app) and the honest
  /// scope it holds, and lets the user NARROW that scope. This is the home of
  /// revoke: granting happens in context (the app asks, you agree), never here;
  /// this page sees, and shrinks, and revokes.
  ///
  /// Scope is rendered from the real ceiling (read vs write, own vs all stay
  /// visible; field and relation detail sit behind an expand), each line
  /// carries its provenance ("Declared at install" vs "You allowed this"), and
  /// a reach into your broad data is a prominent revocable chip while own-data
  /// (a zero-prompt default) is a quiet line. Two pivots: by app, and by data.
  ///
  /// The Settings Tauri bridge is unwired for now, so the store reads a fixture
  /// until it lands. Copy law: no em-dashes, no middot separators; usage is
  /// "not measured yet", never a fabricated "never".
  import { onMount } from "svelte";
  import { Sparkles, AppWindow, ChevronRight, X } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ScopeChip } from "@arlen/ui-kit/components/ui/scope-chip";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    grants,
    grantsLoaded,
    grantsError,
    byApp,
    byData,
    loadGrants,
    revokeScope,
    revokeAllFor,
    type Principal,
    type ScopeLine,
    type RevokeTarget,
  } from "$lib/stores/grants";

  onMount(loadGrants);

  let pivot = $state<"app" | "data">("app");
  const PIVOTS = [
    { value: "app", label: "By app" },
    { value: "data", label: "By data" },
  ];

  const principals = $derived(byApp($grants));
  const assistants = $derived(principals.filter((p) => p.assistant));
  const apps = $derived(principals.filter((p) => !p.assistant));
  const resources = $derived(byData($grants));
  const isEmpty = $derived($grantsLoaded && principals.length === 0);

  // Which scope lines are expanded (by key).
  let expanded = $state<Set<string>>(new Set());
  function toggle(key: string) {
    const next = new Set(expanded);
    next.has(key) ? next.delete(key) : next.add(key);
    expanded = next;
  }

  // The pending revoke, awaiting confirm.
  let pending = $state<{
    title: string;
    message: string;
    confirmLabel: string;
    run: () => Promise<void>;
  } | null>(null);

  function askScope(appLabel: string, line: ScopeLine) {
    pending = {
      title: "Remove access?",
      message: `"${line.text}" will be removed from ${appLabel}. It can ask again if it needs it.`,
      confirmLabel: "Remove",
      run: () => revokeScope(line.revoke),
    };
  }
  function askAll(p: Principal) {
    pending = {
      title: "Remove all access?",
      message: `${p.label} will no longer reach anything on your system. It can ask again if it needs to.`,
      confirmLabel: "Remove all",
      run: () => revokeAllFor(p.lines),
    };
  }
  function askReacher(label: string, line: ScopeLine) {
    askScope(label, line);
  }

  async function onConfirm() {
    if (pending === null) return;
    await pending.run();
    pending = null;
  }
</script>

<Page
  title="App access"
  description="See what each app and the assistant can reach on your system. You remove access here; granting happens in context, when an app asks and you agree."
>
  <SectionGrid>
    <div class="pivot span-full">
      <SegmentedControl
        options={PIVOTS}
        value={pivot}
        ariaLabel="Group access by app or by data"
        onchange={(v) => (pivot = v as "app" | "data")}
      />
    </div>

    {#if $grantsError}
      <Group label="App access" class="span-full">
        <p class="note">Could not read app access. The permission service did not answer.</p>
      </Group>
    {:else if isEmpty}
      <Group label="App access" class="span-full">
        <p class="note">No app holds access to your data.</p>
      </Group>
    {:else if pivot === "app"}
      {#if assistants.length > 0}
        <Group label="The assistant" class="span-full">
          {#each assistants as p (p.appId)}
            {@render principalBlock(p)}
          {/each}
        </Group>
      {/if}
      {#if apps.length > 0}
        <Group label="Apps" class="span-full">
          {#each apps as p, i (p.appId)}
            {#if i > 0}<div class="divider"></div>{/if}
            {@render principalBlock(p)}
          {/each}
        </Group>
      {/if}
    {:else}
      {#each resources as r (r.key)}
        <Group label={r.label} class="span-full">
          {#each r.reachers as reacher (reacher.appId + reacher.line.key)}
            <div class="reacher">
              <span class="lead">
                {#if reacher.assistant}
                  <Sparkles size={16} strokeWidth={1.5} />
                {:else}
                  <AppWindow size={16} strokeWidth={1.5} />
                {/if}
              </span>
              <span class="reacher-who">
                {reacher.label}{#if !reacher.identityVerified}<span class="warn"> unverified</span>{/if}
              </span>
              {@render scopeControl(reacher.label, reacher.line)}
            </div>
          {/each}
        </Group>
      {/each}
    {/if}

    {#if !isEmpty && !$grantsError}
      <p class="usage-note span-full">
        Usage is not measured yet, so this shows what each app can reach, not
        what it has used.
      </p>
    {/if}
  </SectionGrid>
</Page>

{#snippet principalBlock(p: Principal)}
  <div class="principal">
    <div class="p-head">
      <span class="lead">
        {#if p.assistant}
          <Sparkles size={18} strokeWidth={1.5} />
        {:else}
          <AppWindow size={18} strokeWidth={1.5} />
        {/if}
      </span>
      <span class="p-label">{p.label}</span>
      {#if !p.identityVerified}
        <span class="warn">Identity not verified</span>
      {/if}
      <span class="p-spacer"></span>
      <Button variant="ghost" size="sm" onclick={() => askAll(p)}>Remove all</Button>
    </div>
    <div class="lines">
      {#each p.lines as line (line.key)}
        <div class="line">
          <div class="line-main">
            {@render scopeText(p.label, line)}
            <span class="provenance">{line.provenance}</span>
            {#if line.detail.length > 0}
              <button
                type="button"
                class="expand"
                class:open={expanded.has(line.key)}
                aria-label="Show detail"
                onclick={() => toggle(line.key)}
              >
                <ChevronRight size={14} strokeWidth={2} />
              </button>
            {/if}
          </div>
          {#if line.detail.length > 0 && expanded.has(line.key)}
            <ul class="detail">
              {#each line.detail as d (d)}
                <li>{d}</li>
              {/each}
            </ul>
          {/if}
        </div>
      {/each}
    </div>
  </div>
{/snippet}

<!-- The scope, prominent (chip) for a reach into your broad data or quiet (text
     + revoke) for own-data. -->
{#snippet scopeText(appLabel: string, line: ScopeLine)}
  {#if line.chip}
    <ScopeChip label={line.text} onRevoke={() => askScope(appLabel, line)} />
  {:else}
    <span class="quiet">{line.text}</span>
    <button
      type="button"
      class="line-x"
      aria-label={`Remove ${line.text}`}
      onclick={() => askScope(appLabel, line)}
    >
      <X size={13} strokeWidth={2.5} />
    </button>
  {/if}
{/snippet}

<!-- Compact scope for the by-data pivot: the chip or a quiet line with revoke. -->
{#snippet scopeControl(label: string, line: ScopeLine)}
  {#if line.chip}
    <ScopeChip label={line.text} onRevoke={() => askReacher(label, line)} />
  {:else}
    <span class="quiet">{line.text}</span>
    <button
      type="button"
      class="line-x"
      aria-label={`Remove ${line.text}`}
      onclick={() => askReacher(label, line)}
    >
      <X size={13} strokeWidth={2.5} />
    </button>
  {/if}
{/snippet}

<ConfirmDialog
  open={pending !== null}
  title={pending?.title ?? ""}
  message={pending?.message ?? ""}
  confirmLabel={pending?.confirmLabel ?? "Remove"}
  variant="destructive"
  {onConfirm}
  onCancel={() => (pending = null)}
/>

<style>
  .pivot {
    display: flex;
    margin-bottom: 0.25rem;
  }
  .principal {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem 0;
  }
  .divider {
    height: 1px;
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .p-head {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .lead {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .p-label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .p-spacer {
    flex: 1;
  }
  .warn {
    margin-left: 0.375rem;
    font-size: 0.6875rem;
    color: var(--color-warning, #ca8a04);
  }
  /* Scope lines indent under the label, past the leading icon (1.125rem icon +
     the 0.625rem head gap). */
  .lines {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    padding-left: 1.75rem;
  }
  .line {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .line-main {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-height: 1.5rem;
  }
  .quiet {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 72%, transparent);
  }
  .provenance {
    margin-left: auto;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
  .line-x,
  .expand {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
    transition:
      background-color var(--duration-micro, 100ms) var(--ease-out, ease),
      color var(--duration-micro, 100ms) var(--ease-out, ease),
      transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .line-x:hover,
  .expand:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
  }
  .expand.open {
    transform: rotate(90deg);
  }
  .detail {
    margin: 0;
    padding-left: 0.25rem;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
  }
  .detail li {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .reacher {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    padding: 0.5rem 0;
  }
  .reacher-who {
    font-size: 0.8125rem;
    color: var(--foreground);
  }
  .reacher :global(.scope-chip),
  .reacher .quiet {
    margin-left: auto;
  }
  .note {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .usage-note {
    margin: 0;
    padding: 0 0.25rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
