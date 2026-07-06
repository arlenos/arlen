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
  /// Live: `access_grants` reads the whole-system grants; a per-line Remove sends
  /// `revoke_reach` (narrowing-only) and Recently-removed replays `restore_reach`.
  /// A graph reach carries the exact pattern so it revokes here; a required,
  /// system-managed, or non-graph reach (no exact descriptor from the summary yet)
  /// shows a stated reason instead of a button. The store falls back to a fixture
  /// under vite. Copy law: no em-dashes, no middot separators; usage is "not
  /// measured yet", never a fabricated "never".
  import { onMount } from "svelte";
  import {
    Sparkles,
    FolderOpen,
    SquareTerminal,
    SlidersHorizontal,
    ChevronRight,
    Brain,
    Globe,
    Folder,
    Camera,
    Clipboard,
    Bell,
    Zap,
    Workflow,
  } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    grants,
    grantsLoaded,
    grantsError,
    removed,
    byApp,
    byCapability,
    familyGroups,
    loadGrants,
    revokeScope,
    revokeAllFor,
    restore,
    actionNotice,
    type Principal,
    type ScopeLine,
    type RemovedItem,
    type Family,
  } from "$lib/stores/grants";
  import {
    capsules,
    capsulesLoaded,
    loadCapsules,
    revokeCapsule,
    type Capsule,
  } from "$lib/stores/capsules";

  onMount(() => {
    loadGrants();
    loadCapsules();
  });

  // App-first is the default: the surface shows each app (and the assistant, as
  // one row in the same model - the anti-Recall story). "By capability" is the
  // second lens, grouping by what an app can reach.
  let pivot = $state<"app" | "capability">("app");
  const PIVOTS = [
    { value: "app", label: "By app" },
    { value: "capability", label: "By capability" },
  ];

  // The mark for each capability family (the hero anchor of the by-capability
  // view and the subheaders in the by-app view).
  // Keep these in step with the settings nav icons where the system already has
  // one for the same thing: Knowledge Graph -> Brain, Notifications -> Bell,
  // System (Actions) -> Zap, the assistant -> Sparkles.
  const FAMILY_ICONS: Record<Family, typeof Sparkles> = {
    data: Brain,
    network: Globe,
    files: Folder,
    devices: Camera,
    clipboard: Clipboard,
    notifications: Bell,
    system: Zap,
    automation: Workflow,
  };
  function familyIcon(key: string) {
    return FAMILY_ICONS[key as Family] ?? Brain;
  }

  // Known first-party principals get their own mark; everything else falls back
  // to an initial tile. A real per-app icon (the shell's app_index carries one)
  // can replace this once a Settings bridge exposes it.
  const APP_ICONS: Record<string, typeof Sparkles> = {
    "org.arlen.AI1": Sparkles,
    "ai-daemon": Sparkles,
    "org.arlen.AIAgent1": Sparkles,
    "ai-agent": Sparkles,
    "org.arlen.files": FolderOpen,
    "org.arlen.terminal": SquareTerminal,
    "org.arlen.settings": SlidersHorizontal,
  };
  function appIcon(appId: string) {
    return APP_ICONS[appId];
  }

  const principals = $derived(byApp($grants));
  const assistants = $derived(principals.filter((p) => p.assistant));
  const apps = $derived(principals.filter((p) => !p.assistant));
  const resources = $derived(byCapability($grants));
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
    run: () => Promise<RemovedItem[]>;
  } | null>(null);

  function askScope(appLabel: string, line: ScopeLine) {
    pending = {
      title: "Remove access?",
      message: `"${line.text}" will be removed from ${appLabel}. It loses this on its next access, not immediately. It can ask again if it needs it, and you can put it back under Recently removed.`,
      confirmLabel: "Remove",
      run: async () => {
        const it = await revokeScope(line, appLabel);
        return it ? [it] : [];
      },
    };
  }
  function askAll(p: Principal) {
    pending = {
      title: "Remove all access?",
      message: `${p.label} will no longer reach anything on your system. It can ask again if it needs to, and you can put it back under Recently removed.`,
      confirmLabel: "Remove all",
      run: () => revokeAllFor(p.lines, p.label),
    };
  }

  // Revoking a share is terminal: it stops future reads, it cannot pull back a copy
  // the recipient already made, and there is no undo. So the confirm says exactly
  // that, and run returns nothing (no undo snackbar).
  function askRevokeCapsule(c: Capsule) {
    pending = {
      title: "Revoke this share?",
      message: `"${c.label}" (shared with ${c.audience}) stops being readable on its next access. It cannot pull back a copy the recipient already made, and this cannot be undone.`,
      confirmLabel: "Revoke",
      run: async () => {
        await revokeCapsule(c.id);
        return [];
      },
    };
  }

  async function onConfirm() {
    if (pending === null) return;
    const items = await pending.run();
    pending = null;
    if (items.length > 0) showUndo(items);
  }

  // The immediate undo after a removal: a brief snackbar that reinstates exactly
  // what was just taken away.
  let undo = $state<{ items: RemovedItem[]; text: string } | null>(null);
  let undoTimer: ReturnType<typeof setTimeout> | null = null;
  function showUndo(items: RemovedItem[]) {
    const text =
      items.length === 1
        ? `Removed ${items[0].text}.`
        : `Removed ${items.length} from ${items[0].appLabel}.`;
    undo = { items, text };
    if (undoTimer) clearTimeout(undoTimer);
    undoTimer = setTimeout(() => (undo = null), 7000);
  }
  async function doUndo() {
    if (undo === null) return;
    const items = undo.items;
    if (undoTimer) clearTimeout(undoTimer);
    undo = null;
    for (const it of items) await restore(it);
  }

  // The reach summary for a by-capability row: the full sentence, since the
  // group now names the capability family (not the specific data type), so the
  // object still has to say what is reached.
  function howText(line: ScopeLine): string {
    return line.text;
  }

  // The short muted marker shown where a Remove button cannot be, with the reason
  // stated (settled model: explained before the click, no tooltip).
  function revokeLabel(line: ScopeLine): string {
    if (line.required) return "Required";
    if (line.systemManaged) return "System-managed";
    return "Not yet revocable";
  }

  // Clear a revoke/restore failure notice after a moment.
  $effect(() => {
    if ($actionNotice) {
      const t = setTimeout(() => actionNotice.set(null), 5000);
      return () => clearTimeout(t);
    }
  });
</script>

<Page
  title="App access"
  description="Everything that can reach your data, and everything you share out. You remove access or revoke a share here; an app asks when it needs more."
>
  <SectionGrid>
    <div class="pivot span-full">
      <SegmentedControl
        options={PIVOTS}
        value={pivot}
        ariaLabel="Group access by app or by data"
        onchange={(v) => (pivot = v as "app" | "capability")}
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
        {@const FamilyIcon = familyIcon(r.key)}
        {@const appCount = new Set(r.reachers.map((x) => x.appId)).size}
        <Group class="span-full">
          <div class="data-head">
            <span class="data-icon"><FamilyIcon size={16} strokeWidth={1.75} /></span>
            <span class="data-name">{r.label}</span>
            <span class="data-count">
              {appCount}
              {appCount === 1 ? "app" : "apps"}
            </span>
          </div>
          <div class="reacher-list">
            {#each r.reachers as reacher (reacher.appId + reacher.line.key)}
              {@render avatar(reacher.appId, reacher.label, 24)}
              <span class="who">
                {reacher.label}{#if !reacher.identityVerified}<span class="warn">unverified</span>{/if}
              </span>
              <span class="how" class:dim={reacher.line.own}>{howText(reacher.line)}</span>
              <span class="reacher-prov">{reacher.line.provenance}</span>
              {#if reacher.line.revoke.enabled}
                <button
                  type="button"
                  class="remove"
                  aria-label={`Remove ${reacher.label} ${reacher.line.text}`}
                  onclick={() => askScope(reacher.label, reacher.line)}
                >
                  Remove
                </button>
              {:else}
                <span class="remove-off">{revokeLabel(reacher.line)}</span>
              {/if}
            {/each}
          </div>
        </Group>
      {/each}
    {/if}

    {#if $capsulesLoaded}
      <Group label="Shared context" class="span-full">
        {#if $capsules.length > 0}
          <div class="cap-list">
            {#each $capsules as c (c.id)}
              <span class="cap-what">
                <span class="cap-label">{c.label}</span>
                <span class="cap-scope">{c.scope}</span>
              </span>
              <span class="cap-who">{c.audience}</span>
              <span class="cap-when">{c.expiresAt}</span>
              {#if c.state === "active"}
                <span class="cap-reads">{c.readsLeft} {c.readsLeft === 1 ? "read" : "reads"} left</span>
                <button
                  type="button"
                  class="remove"
                  aria-label={`Revoke the share "${c.label}"`}
                  onclick={() => askRevokeCapsule(c)}
                >
                  Revoke
                </button>
              {:else}
                <span class="cap-reads dim">{c.state === "expired" ? "expired" : "no reads left"}</span>
                <span class="remove-off">closed</span>
              {/if}
            {/each}
          </div>
          <p class="note">
            Revoking a share stops any further reads. It cannot pull back a copy
            the recipient already made, and a share is a snapshot from when you
            made it, not a live feed.
          </p>
        {:else}
          <p class="note">You have not shared any context.</p>
        {/if}
      </Group>
    {/if}

    {#if $removed.length > 0}
      <Group label="Recently removed" class="span-full">
        <div class="removed-list">
          {#each $removed as it (it.id)}
            {@render avatar(it.appId, it.appLabel, 24)}
            <span class="who">{it.appLabel}</span>
            <span class="how">{it.text}</span>
            <button type="button" class="restore" onclick={() => restore(it)}>
              Restore
            </button>
          {/each}
        </div>
      </Group>
    {/if}

    {#if !isEmpty && !$grantsError}
      <p class="usage-note span-full">
        Usage is not measured yet, so this shows what each app can reach, not
        what it has used.
      </p>
    {/if}
  </SectionGrid>
</Page>

{#if undo}
  <div class="snackbar" role="status">
    <span class="snack-text">{undo.text}</span>
    <button type="button" class="snack-undo" onclick={doUndo}>Undo</button>
  </div>
{/if}

{#if $actionNotice}
  <div class="snackbar" role="status">
    <span class="snack-text">{$actionNotice}</span>
    <button type="button" class="snack-undo" onclick={() => actionNotice.set(null)}>Dismiss</button>
  </div>
{/if}

{#snippet avatar(appId: string, label: string, size: number)}
  {@const Icon = appIcon(appId)}
  <span class="avatar" style={`width:${size}px;height:${size}px`}>
    {#if Icon}
      <Icon size={size * 0.6} strokeWidth={1.75} />
    {:else}
      <span class="avatar-initial" style={`font-size:${size * 0.42}px`}>
        {label.charAt(0).toUpperCase()}
      </span>
    {/if}
  </span>
{/snippet}

{#snippet principalBlock(p: Principal)}
  <div class="principal">
    <div class="p-head">
      {@render avatar(p.appId, p.label, 28)}
      <span class="p-label">{p.label}</span>
      {#if !p.identityVerified}<span class="warn">unverified</span>{/if}
      <span class="p-spacer"></span>
      <button type="button" class="remove" onclick={() => askAll(p)}>Remove all</button>
    </div>
    {#each familyGroups(p.lines) as fam (fam.key)}
      {@const FamIcon = familyIcon(fam.key)}
      <div class="fam-sub">
        <span class="fam-sub-icon"><FamIcon size={13} strokeWidth={1.75} /></span>
        <span class="fam-sub-label">{fam.label}</span>
      </div>
      <div class="lines">
        {#each fam.lines as line (line.key)}
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
          {#if line.revoke.enabled}
            <button
              type="button"
              class="remove"
              aria-label={`Remove ${line.text}`}
              onclick={() => askScope(p.label, line)}
            >
              Remove
            </button>
          {:else}
            <span class="remove-off">{revokeLabel(line)}</span>
          {/if}
          {#if line.detail.length > 0 && expanded.has(line.key)}
            <ul class="detail">
              {#each line.detail as d (d)}
                <li>{d}</li>
              {/each}
            </ul>
          {/if}
        {/each}
      </div>
    {/each}
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
  .avatar-initial {
    font-weight: 600;
    line-height: 1;
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

  /* Family subheader inside an app block: a quiet category label above that
     family's lines, indented to the label edge. */
  .fam-sub {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding-left: calc(28px + 0.625rem);
    margin-top: 0.625rem;
    margin-bottom: 0.25rem;
  }
  .fam-sub-icon {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .fam-sub-label {
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
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
  /* A stated reason where a Remove cannot be: required, system-managed, or a reach
     without an exact revoke descriptor yet. Quiet, not an action. */
  .remove-off {
    justify-self: end;
    flex-shrink: 0;
    padding: 0.125rem 0.25rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 32%, transparent);
    white-space: nowrap;
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

  /* By-data hero: the kind of data is the anchor, larger than an app name, with
     its mark and how many apps can reach it. */
  .data-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .data-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .data-name {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .data-count {
    margin-left: auto;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  /* By-data: an aligned grid of the apps that reach this data. Avatar, who,
     then the "how" and Remove as their own columns so they line up down the
     list. */
  .reacher-list {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) max-content max-content max-content;
    align-items: center;
    column-gap: 0.625rem;
    row-gap: 0.75rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  /* Provenance shows only where it is notable (a location you granted in
     context); a declared reach is the implied default, left blank so the column
     never repeats "declared" down the list. */
  .reacher-prov {
    justify-self: end;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
    white-space: nowrap;
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

  /* Recently removed: the same aligned grid, with a quiet Restore that puts back
     exactly what was taken (never a fresh grant). */
  .removed-list {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) max-content max-content;
    align-items: center;
    column-gap: 0.625rem;
    row-gap: 0.75rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .restore {
    justify-self: end;
    flex-shrink: 0;
    border: none;
    background: transparent;
    padding: 0.125rem 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    transition: color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .restore:hover {
    color: var(--color-accent, var(--foreground));
  }

  /* Immediate-undo snackbar: a brief bar pinned to the viewport bottom. */
  .snackbar {
    position: fixed;
    left: 50%;
    bottom: 1.5rem;
    transform: translateX(-50%);
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.625rem 0.75rem 0.625rem 1rem;
    border-radius: var(--radius-card, 12px);
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    background: var(--popover, var(--card, #1f1f23));
    box-shadow: var(--shadow-lg, 0 8px 24px rgba(0, 0, 0, 0.3));
  }
  .snack-text {
    font-size: 0.8125rem;
    color: var(--foreground);
  }
  .snack-undo {
    border: none;
    background: transparent;
    padding: 0.125rem 0.375rem;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-accent, var(--foreground));
    cursor: pointer;
  }
  .snack-undo:hover {
    text-decoration: underline;
  }

  /* Shared context (outbound): the same aligned-grid language as the reachers,
     read as a sentence - what, with whom, until when, how many reads left. */
  .cap-list {
    display: grid;
    grid-template-columns: minmax(0, 1fr) max-content max-content max-content max-content;
    align-items: center;
    column-gap: 1rem;
    row-gap: 0.75rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .cap-what {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }
  .cap-label {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .cap-scope {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cap-who {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    white-space: nowrap;
  }
  .cap-when {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
  .cap-reads {
    justify-self: end;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .cap-reads.dim {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
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
