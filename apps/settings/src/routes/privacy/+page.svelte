<script lang="ts">
  /// App access - the system-wide capability browser (living-capability-graph.md
  /// §6). The surface reads as a plain statement about the user's data: who can
  /// reach it, and how. Granting happens in context (the app asks, you agree),
  /// never here; this page sees, shrinks, and revokes.
  ///
  /// Each reach is a sentence - a quiet verb over the emphasized object, because
  /// what matters is the user's data, not the app. Read vs write and own vs all
  /// stay visible; field and relation detail sit behind an expand. Each line
  /// carries its provenance ("declared at install" vs "you allowed this"); a
  /// reach into your broad data is emphasized, own-data (a zero-prompt default)
  /// is dimmed. Two pivots: by app, and by data.
  ///
  /// The Settings Tauri bridge is unwired for now, so the store reads a fixture
  /// until it lands. Copy law: no em-dashes, no middot separators; usage is
  /// "not measured yet", never a fabricated "never".
  import { onMount } from "svelte";
  import { Sparkles, AppWindow, ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
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

  let expanded = $state<Set<string>>(new Set());
  function toggle(key: string) {
    const next = new Set(expanded);
    next.has(key) ? next.delete(key) : next.add(key);
    expanded = next;
  }

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

  async function onConfirm() {
    if (pending === null) return;
    await pending.run();
    pending = null;
  }

  // The reach summary for a by-data row. The group header already names the
  // data type, so a typed reach shows just the verb (plus the own qualifier); a
  // consent path has no type header, so it shows the full "access to <path>".
  function howText(line: ScopeLine): string {
    if (line.entityType === null) return line.text;
    return line.own ? `${line.verb} its own only` : line.verb;
  }
</script>

<Page
  title="App access"
  description="Who can reach your data. You remove access here; an app asks when it needs more."
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
          {#each apps as p (p.appId)}
            {@render principalBlock(p)}
          {/each}
        </Group>
      {/if}
    {:else}
      {#each resources as r (r.key)}
        <Group label={r.label} class="span-full">
          <div class="reacher-list">
            {#each r.reachers as reacher (reacher.appId + reacher.line.key)}
              {@render avatar(reacher.assistant, 24)}
              <span class="who">
                {reacher.label}{#if !reacher.identityVerified}<span class="warn">unverified</span>{/if}
              </span>
              <span class="how" class:dim={reacher.line.own}>{howText(reacher.line)}</span>
              <button
                type="button"
                class="remove"
                aria-label={`Remove ${reacher.label} ${reacher.line.text}`}
                onclick={() => askScope(reacher.label, reacher.line)}
              >
                Remove
              </button>
            {/each}
          </div>
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

{#snippet avatar(assistant: boolean, size: number)}
  <span class="avatar" style={`width:${size}px;height:${size}px`}>
    {#if assistant}
      <Sparkles size={size * 0.6} strokeWidth={1.75} />
    {:else}
      <AppWindow size={size * 0.6} strokeWidth={1.75} />
    {/if}
  </span>
{/snippet}

{#snippet principalBlock(p: Principal)}
  <div class="principal">
    <div class="p-head">
      {@render avatar(p.assistant, 28)}
      <span class="p-label">{p.label}</span>
      {#if !p.identityVerified}<span class="warn">unverified</span>{/if}
      <span class="p-spacer"></span>
      <button type="button" class="remove" onclick={() => askAll(p)}>Remove all</button>
    </div>
    <div class="lines">
      {#each p.lines as line (line.key)}
        <span class="verb" class:dim={line.own}>{line.verb}</span>
        <span class="object" class:dim={line.own}>
          {line.object}
          {#if line.detail.length > 0}
            <button
              type="button"
              class="expand"
              class:open={expanded.has(line.key)}
              aria-label="Show detail"
              onclick={() => toggle(line.key)}
            >
              <ChevronRight size={13} strokeWidth={2} />
            </button>
          {/if}
        </span>
        <span class="prov" class:dim={line.own}>{line.provenance}</span>
        <button
          type="button"
          class="remove"
          aria-label={`Remove ${line.text}`}
          onclick={() => askScope(p.label, line)}
        >
          Remove
        </button>
        {#if line.detail.length > 0 && expanded.has(line.key)}
          <ul class="detail">
            {#each line.detail as d (d)}
              <li>{d}</li>
            {/each}
          </ul>
        {/if}
      {/each}
    </div>
  </div>
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
  /* The app identity tile: a calm slot for the icon, forward-compatible with a
     real app icon replacing the glyph. */
  .avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }

  /* Match the Row inset (Group has no padding of its own; each direct child
     provides it, and the card draws the divider between children). */
  .principal {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .p-head {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .p-label {
    font-size: 0.8125rem;
    font-weight: 600;
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

  /* Sentence lines as an aligned grid, indented under the label past the 28px
     avatar + head gap. The verb is right-aligned so the data (the object) forms
     a clean scannable column; provenance and Remove are their own columns. */
  .lines {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) max-content max-content;
    align-items: baseline;
    column-gap: 0.75rem;
    row-gap: 0.5rem;
    padding-left: calc(28px + 0.625rem);
  }
  /* The reach as a sentence: the verb quiet, the object (the user's data) the
     emphasized word. Own-data dims the line. */
  .verb {
    justify-self: end;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .object {
    justify-self: start;
    display: inline-flex;
    align-items: baseline;
    gap: 0.375rem;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .prov {
    justify-self: start;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    white-space: nowrap;
  }
  .dim {
    opacity: 0.6;
  }

  /* "Remove" is quiet by default and firms up on hover; a calm tidy action, not
     an alarm. */
  .remove {
    justify-self: end;
    flex-shrink: 0;
    border: none;
    background: transparent;
    padding: 0.125rem 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
    transition: color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .remove:hover {
    color: var(--color-error, #dc2626);
  }

  .expand {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.125rem;
    height: 1.125rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    cursor: pointer;
    transition:
      color var(--duration-micro, 100ms) var(--ease-out, ease),
      transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .expand:hover {
    color: var(--foreground);
  }
  .expand.open {
    transform: rotate(90deg);
  }
  /* Detail sits under the object column, not the verb. */
  .detail {
    grid-column: 2 / -1;
    margin: -0.125rem 0 0.125rem;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
  }
  .detail li {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  /* By-data: an aligned grid of the apps that reach this data. Avatar, who,
     then the "how" and Remove as their own columns so they line up down the
     list. */
  .reacher-list {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) max-content max-content;
    align-items: center;
    column-gap: 0.625rem;
    row-gap: 0.75rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .who {
    justify-self: start;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .how {
    justify-self: end;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .how.dim {
    opacity: 0.75;
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
