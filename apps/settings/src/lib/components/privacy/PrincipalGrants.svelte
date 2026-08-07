<script lang="ts">
  /// One app's access block: the identity head, then each capability family's
  /// reach lines as sentences (quiet verb over the emphasized object, since what
  /// matters is the user's data, not the app), provenance, and the per-line
  /// Remove. One source for the privacy browser's by-app pivot AND the per-app
  /// settings page. `split` demotes the required reaches into their own run
  /// below the revocable ones with a stated reason (the per-app page's order);
  /// `showHead` off drops the identity row where the page above already names
  /// the app.
  import { ChevronRight } from "lucide-svelte";
  import { familyGroups, type Principal, type ScopeLine } from "$lib/stores/grants";
  import { familyIcon } from "./familyIcons";
  import AppAvatar from "./AppAvatar.svelte";
  import { t } from "$lib/i18n/messages";

  let {
    principal,
    split = false,
    showHead = true,
    onRemoveScope,
    onRemoveAll,
  }: {
    principal: Principal;
    split?: boolean;
    showHead?: boolean;
    onRemoveScope: (appLabel: string, line: ScopeLine) => void;
    onRemoveAll?: (p: Principal) => void;
  } = $props();

  let expanded = $state<Set<string>>(new Set());
  function toggle(key: string) {
    const next = new Set(expanded);
    next.has(key) ? next.delete(key) : next.add(key);
    expanded = next;
  }

  // The short muted marker shown where a Remove button cannot be, with the
  // reason stated (settled model: explained before the click, no tooltip).
  function revokeLabel(line: ScopeLine): string {
    if (line.required) return $t("s.priv.required");
    if (line.systemManaged) return $t("s.priv.systemManaged");
    return $t("s.priv.notRevocable");
  }

  const mainLines = $derived(split ? principal.lines.filter((l) => !l.required) : principal.lines);
  const requiredLines = $derived(split ? principal.lines.filter((l) => l.required) : []);
</script>

{#snippet lineRun(lines: ScopeLine[], requiredRun: boolean)}
  {#each familyGroups(lines) as fam (fam.key)}
    {@const FamIcon = familyIcon(fam.key)}
    <div class="fam-sub" class:flush={!showHead}>
      <span class="fam-sub-icon"><FamIcon size={13} strokeWidth={1.75} /></span>
      <span class="fam-sub-label">{$t(fam.label)}</span>
    </div>
    <div class="lines" class:flush={!showHead}>
      {#each fam.lines as line (line.key)}
        <span class="verb" class:dim={line.own}>{line.verb}</span>
        <span class="object" class:dim={line.own}>
          {line.object}
          {#if line.detail.length > 0}
            <button
              type="button"
              class="expand"
              class:open={expanded.has(line.key)}
              aria-label={$t("s.priv.showDetail")}
              onclick={() => toggle(line.key)}
            >
              <ChevronRight size={13} strokeWidth={2} />
            </button>
          {/if}
        </span>
        <span class="prov" class:dim={line.own}>{$t(line.provenance.id, line.provenance.params)}</span>
        {#if requiredRun}
          <!-- The run's intro already states why these cannot be removed; a
               per-line "Required" marker would just repeat it down the column. -->
          <span class="remove-off"></span>
        {:else if line.revoke.enabled}
          <button
            type="button"
            class="remove"
            aria-label={$t("s.priv.removeLineAria", { what: line.text })}
            onclick={() => onRemoveScope(principal.label, line)}
          >
            {$t("s.priv.remove")}
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
{/snippet}

<div class="principal">
  {#if showHead}
    <div class="p-head">
      <AppAvatar appId={principal.appId} label={principal.label} size={28} />
      <span class="p-label">{principal.label}</span>
      {#if !principal.identityVerified}<span class="warn">{$t("s.priv.unverified")}</span>{/if}
      <span class="p-spacer"></span>
      {#if onRemoveAll}
        <button type="button" class="remove" onclick={() => onRemoveAll?.(principal)}>{$t("s.priv.removeAll")}</button>
      {/if}
    </div>
  {/if}
  {@render lineRun(mainLines, false)}
  {#if requiredLines.length > 0}
    <p class="req-note" class:flush={!showHead}>{$t("s.apps.requiredNote", { app: principal.label })}</p>
    {@render lineRun(requiredLines, true)}
  {/if}
</div>

<style>
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
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--foreground);
  }
  .p-spacer {
    flex: 1;
  }
  .warn {
    margin-inline-start: 0.375rem;
    font-size: var(--text-2xs);
    color: var(--color-warning, #ca8a04);
  }

  /* Family subheader inside an app block: a quiet category label above that
     family's lines, indented to the label edge. */
  .fam-sub {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding-inline-start: calc(28px + 0.625rem);
    margin-top: 0.625rem;
    margin-bottom: 0.25rem;
  }
  .fam-sub-icon {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .fam-sub-label {
    font-size: var(--text-2xs);
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
    padding-inline-start: calc(28px + 0.625rem);
  }
  /* Headless (the per-app page names the app above): the block sits flush with
     the section inset instead of hanging under an absent avatar. */
  .fam-sub.flush,
  .lines.flush,
  .req-note.flush {
    padding-inline-start: 0;
  }
  /* The reach as a sentence: the verb quiet, the object (the user's data) the
     emphasized word. Own-data dims the line. */
  .verb {
    justify-self: end;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .object {
    justify-self: start;
    display: inline-flex;
    align-items: baseline;
    gap: 0.375rem;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .prov {
    justify-self: start;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    white-space: nowrap;
  }
  .dim {
    opacity: 0.6;
  }

  /* The demoted required run's intro: the one place that says why the lines
     below carry no Remove. */
  .req-note {
    margin: 0.375rem 0 0;
    padding-inline-start: calc(28px + 0.625rem);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
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
  /* A stated reason where a Remove cannot be: required, system-managed, or a
     reach without an exact revoke descriptor yet. Quiet, not an action. */
  .remove-off {
    justify-self: end;
    flex-shrink: 0;
    padding: 0.125rem 0.25rem;
    font-size: var(--text-xs);
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
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
