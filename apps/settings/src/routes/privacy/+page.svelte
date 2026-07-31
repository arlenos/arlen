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
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { Radar } from "lucide-svelte";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    grants,
    grantsLoaded,
    grantsError,
    grantsMocked,
    removed,
    byApp,
    byCapability,
    loadGrants,
    revokeScope,
    revokeAllFor,
    restore,
    actionNotice,
    type Principal,
    type ScopeLine,
    type RemovedItem,
  } from "$lib/stores/grants";
  import PrincipalGrants from "$lib/components/privacy/PrincipalGrants.svelte";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import { familyIcon } from "$lib/components/privacy/familyIcons";
  import {
    capsules,
    capsulesLoaded,
    capsulesMocked,
    loadCapsules,
    revokeCapsule,
    type Capsule,
  } from "$lib/stores/capsules";
  import { t } from "$lib/i18n/messages";

  onMount(() => {
    loadGrants();
    loadCapsules();
  });

  // App-first is the default: the surface shows each app (and the assistant, as
  // one row in the same model - the anti-Recall story). "By capability" is the
  // second lens, grouping by what an app can reach.
  let pivot = $state<"app" | "capability">("app");
  const PIVOTS = $derived([
    { value: "app", label: $t("s.priv.byApp") },
    { value: "capability", label: $t("s.priv.byCapability") },
  ]);

  const principals = $derived(byApp($grants));
  const assistants = $derived(principals.filter((p) => p.assistant));
  const apps = $derived(principals.filter((p) => !p.assistant));
  const resources = $derived(byCapability($grants));
  const isEmpty = $derived($grantsLoaded && principals.length === 0);

  let pending = $state<{
    title: string;
    message: string;
    confirmLabel: string;
    run: () => Promise<RemovedItem[]>;
  } | null>(null);

  function askScope(appLabel: string, line: ScopeLine) {
    pending = {
      title: $t("s.priv.askScope.title"),
      message: $t("s.priv.askScope.msg", { what: line.text, app: appLabel }),
      confirmLabel: $t("s.priv.remove"),
      run: async () => {
        const it = await revokeScope(line, appLabel);
        return it ? [it] : [];
      },
    };
  }
  function askAll(p: Principal) {
    pending = {
      title: $t("s.priv.askAll.title"),
      message: $t("s.priv.askAll.msg", { app: p.label }),
      confirmLabel: $t("s.priv.removeAll"),
      run: () => revokeAllFor(p.lines, p.label),
    };
  }

  // Revoking a share is terminal: it stops future reads, it cannot pull back a copy
  // the recipient already made, and there is no undo. So the confirm says exactly
  // that, and run returns nothing (no undo snackbar).
  function askRevokeCapsule(c: Capsule) {
    pending = {
      title: $t("s.priv.askCapsule.title"),
      message: $t("s.priv.askCapsule.msg", { label: c.label, audience: c.audience }),
      confirmLabel: $t("s.priv.revoke"),
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
        ? $t("s.priv.undoOne", { what: items[0].text })
        : $t("s.priv.undoMany", { count: items.length, app: items[0].appLabel });
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
  // stated (settled model: explained before the click, no tooltip). The by-app
  // pivot's copy of this lives inside PrincipalGrants.
  function revokeLabel(line: ScopeLine): string {
    if (line.required) return $t("s.priv.required");
    if (line.systemManaged) return $t("s.priv.systemManaged");
    return $t("s.priv.notRevocable");
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
  title={$t("s.priv.title")}
  description={$t("s.priv.desc")}
>
  <SectionGrid>
    <div class="pivot span-full">
      <SegmentedControl
        options={PIVOTS}
        value={pivot}
        ariaLabel={$t("s.priv.pivotAria")}
        onchange={(v) => (pivot = v as "app" | "capability")}
      />
    </div>

    {#if $grantsMocked}
      <!-- Said before any grant is read: this list invents access that does not
           exist AND omits access that does, and it is the surface a user trusts
           to answer "what can reach my data". -->
      <p class="note span-full">{$t("s.priv.sample")}</p>
    {/if}

    <div class="span-full">
      <LinkCard href="/privacy/physical" title={$t("s.sent.link")} description={$t("s.sent.linkDesc")}>
        {#snippet icon()}<Radar size={20} strokeWidth={1.75} />{/snippet}
      </LinkCard>
    </div>

    {#if $grantsError}
      <Section label={$t("s.priv.title")} class="span-full">
        <p class="note">{$t("s.priv.readError")}</p>
      </Section>
    {:else if isEmpty}
      <Section label={$t("s.priv.title")} class="span-full">
        <p class="note">{$t("s.priv.empty")}</p>
      </Section>
    {:else if pivot === "app"}
      {#if assistants.length > 0}
        <Section label={$t("s.priv.assistant")} class="span-full">
          {#each assistants as p (p.appId)}
            <PrincipalGrants principal={p} onRemoveScope={askScope} onRemoveAll={askAll} />
          {/each}
        </Section>
      {/if}
      {#if apps.length > 0}
        <Section label={$t("s.priv.apps")} class="span-full">
          {#each apps as p (p.appId)}
            <PrincipalGrants principal={p} onRemoveScope={askScope} onRemoveAll={askAll} />
          {/each}
        </Section>
      {/if}
    {:else}
      {#each resources as r (r.key)}
        {@const FamilyIcon = familyIcon(r.key)}
        {@const appCount = new Set(r.reachers.map((x) => x.appId)).size}
        <Section class="span-full">
          <div class="data-head">
            <span class="data-icon"><FamilyIcon size={16} strokeWidth={1.75} /></span>
            <span class="data-name">{r.label}</span>
            <span class="data-count">
              {$t("s.priv.appCount", { count: appCount })}
            </span>
          </div>
          <div class="reacher-list">
            {#each r.reachers as reacher (reacher.appId + reacher.line.key)}
              <AppAvatar appId={reacher.appId} label={reacher.label} size={24} />
              <span class="who">
                {reacher.label}{#if !reacher.identityVerified}<span class="warn">{$t("s.priv.unverified")}</span>{/if}
              </span>
              <span class="how" class:dim={reacher.line.own}>{howText(reacher.line)}</span>
              <span class="reacher-prov">{reacher.line.provenance}</span>
              {#if reacher.line.revoke.enabled}
                <button
                  type="button"
                  class="remove"
                  aria-label={$t("s.priv.removeAria", { app: reacher.label, what: reacher.line.text })}
                  onclick={() => askScope(reacher.label, reacher.line)}
                >
                  {$t("s.priv.remove")}
                </button>
              {:else}
                <span class="remove-off">{revokeLabel(reacher.line)}</span>
              {/if}
            {/each}
          </div>
        </Section>
      {/each}
    {/if}

    {#if $capsulesLoaded}
      <Section label={$t("s.priv.sharedContext")} class="span-full">
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
                <span class="cap-reads">{$t("s.priv.readsLeft", { count: c.readsLeft })}</span>
                <button
                  type="button"
                  class="remove"
                  aria-label={$t("s.priv.revokeShareAria", { label: c.label })}
                  onclick={() => askRevokeCapsule(c)}
                >
                  {$t("s.priv.revoke")}
                </button>
              {:else}
                <span class="cap-reads dim">{c.state === "expired" ? $t("s.priv.expired") : $t("s.priv.noReadsLeft")}</span>
                <span class="remove-off">{$t("s.priv.closed")}</span>
              {/if}
            {/each}
          </div>
          {#if $capsulesMocked}
            <!-- Invented shares read as data actually sent out and still readable. -->
            <p class="note">{$t("s.priv.sampleShares")}</p>
          {/if}
          <p class="note">
            {$t("s.priv.shareNote")}
          </p>
        {:else}
          <p class="note">{$t("s.priv.noShares")}</p>
        {/if}
      </Section>
    {/if}

    {#if $removed.length > 0}
      <Section label={$t("s.priv.recentlyRemoved")} class="span-full">
        <div class="removed-list">
          {#each $removed as it (it.id)}
            <AppAvatar appId={it.appId} label={it.appLabel} size={24} />
            <span class="who">{it.appLabel}</span>
            <span class="how">{it.text}</span>
            <button type="button" class="restore" onclick={() => restore(it)}>
              {$t("s.priv.restore")}
            </button>
          {/each}
        </div>
      </Section>
    {/if}

    {#if !isEmpty && !$grantsError}
      <p class="usage-note span-full">
        {$t("s.priv.usageNote")}
      </p>
    {/if}
  </SectionGrid>
</Page>

{#if undo}
  <div class="snackbar" role="status">
    <span class="snack-text">{undo.text}</span>
    <button type="button" class="snack-undo" onclick={doUndo}>{$t("s.priv.undo")}</button>
  </div>
{/if}

{#if $actionNotice}
  <div class="snackbar" role="status">
    <span class="snack-text">{$actionNotice}</span>
    <button type="button" class="snack-undo" onclick={() => actionNotice.set(null)}>{$t("s.priv.dismiss")}</button>
  </div>
{/if}

<ConfirmDialog
  open={pending !== null}
  title={pending?.title ?? ""}
  message={pending?.message ?? ""}
  confirmLabel={pending?.confirmLabel ?? $t("s.priv.remove")}
  variant="destructive"
  {onConfirm}
  onCancel={() => (pending = null)}
/>

<style>
  .pivot {
    display: flex;
    margin-bottom: 0.25rem;
  }
  .warn {
    margin-inline-start: 0.375rem;
    font-size: var(--text-2xs);
    color: var(--color-warning, #ca8a04);
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
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 32%, transparent);
    white-space: nowrap;
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
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--foreground);
  }
  .data-count {
    margin-inline-start: auto;
    font-size: var(--text-xs);
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
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
    white-space: nowrap;
  }
  .who {
    justify-self: start;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .how {
    justify-self: end;
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
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
    font-size: var(--text-sm);
    color: var(--foreground);
  }
  .snack-undo {
    border: none;
    background: transparent;
    padding: 0.125rem 0.375rem;
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--foreground);
  }
  .cap-scope {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cap-who {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    white-space: nowrap;
  }
  .cap-when {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
  .cap-reads {
    justify-self: end;
    font-size: var(--text-xs);
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
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .usage-note {
    margin: 0;
    padding: 0 0.25rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
