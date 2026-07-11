<script lang="ts">
  /// The unified consent dialog (system-dialog-plan.md): one polymorphic surface
  /// every permission request routes into, the sibling of the App-access page.
  /// Mounted once in +layout, inert when nothing is pending.
  ///
  /// The frame is common to every request - the attested requester (the shown
  /// identity IS the grant recipient), the plain-language ask, the concrete
  /// scope - but the WEIGHT scales with the stakes, carried by a single accent
  /// edge (none / amber caution / red danger) so nothing is said twice. A benign
  /// grant is calm and neutral; a caution-class ask wears an amber edge; a
  /// permanent delete wears a red edge, names every file it destroys, and can
  /// only be answered by a deliberate press-and-hold. Deny is always
  /// first-class; the least-privilege default is "once". This makes the
  /// dangerous request impossible to dispatch with the same reflex as the
  /// routine one.
  import { onMount } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Avatar, AvatarFallback } from "@arlen/ui-kit/components/ui/avatar";
  import { AlertTriangle, Send, Trash2, ShieldCheck } from "lucide-svelte";
  import { current, resolve, pollConsent, type PendingView } from "$lib/stores/consent";

  onMount(() => {
    void pollConsent();
  });

  const NAMES: Record<string, string> = {
    "org.arlen.files": "Files",
    "org.arlen.installd": "Software install",
    "com.example.notes": "Notes",
    "com.example.mail": "Mail",
  };
  function friendly(id: string): string {
    const seg = id.split(".").pop() ?? id;
    return NAMES[id] ?? seg.charAt(0).toUpperCase() + seg.slice(1);
  }

  // The single semantic accent the surface wears. Danger is reserved for the
  // truly irreversible; caution for the classes that reach outside their sandbox
  // (send, admin, run, install); everything else stays neutral and calm.
  type Tone = "danger" | "caution" | "neutral";
  function toneOf(p: PendingView): Tone {
    // Red is reserved for destroying data you cannot get back. Reaching outside
    // the sandbox (send, admin, run, install) is caution, not destruction - even
    // when the act itself cannot be recalled, like an email once sent.
    if (p.class === "destructive" && p.reversibility === "irreversible") return "danger";
    if (
      p.class === "external_send" ||
      p.class === "elevated_privilege" ||
      p.class === "exec_confined" ||
      p.class === "install"
    )
      return "caution";
    if (p.reversibility === "irreversible") return "danger";
    return "neutral";
  }
  function scopeLabel(p: PendingView): string {
    if (p.class === "external_send") return "To";
    if (p.class === "destructive") return "Target";
    return "Scope";
  }
  // Habituation defeat: with a single target the confirm names it, so the button
  // reads differently each time and cannot be answered from muscle memory. With
  // several, the list above already names them - the button stays plain rather
  // than repeat the count.
  function holdLabel(p: PendingView): string {
    if (p.targets && p.targets.length === 1) return `Hold to delete ${p.targets[0].name}`;
    return "Hold to delete";
  }

  function deny(p: PendingView) {
    void resolve(p.id, "denied");
  }
  function allowOnce(p: PendingView) {
    void resolve(p.id, "allowed_once");
  }
  function allowRemember(p: PendingView) {
    void resolve(p.id, "allowed_remembered");
  }

  // Hold-to-confirm for the destructive class: a press-and-hold fills the button
  // over ~1.2s, then fires. Releasing early cancels. The confirm delay is the
  // anti-accident affordance for the one class that cannot be undone.
  let holding = $state(false);
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  function holdStart(p: PendingView) {
    holding = true;
    holdTimer = setTimeout(() => {
      holding = false;
      allowOnce(p);
    }, 1200);
  }
  function holdEnd() {
    holding = false;
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
  }
</script>

{#if $current}
  {@const p = $current}
  <!-- The gate is reversibility, not impact (system-dialog-plan.md): reversible
       actions get the generous remember (it carries autonomous authority);
       only the genuinely irreversible confirm per instance. Destructive is NOT
       automatically irreversible - move-to-Trash is reversible. -->
  {@const tone = toneOf(p)}
  {@const holdDestructive = p.class === "destructive" && p.reversibility === "irreversible"}
  {@const standingElsewhere = p.class === "external_send" || p.class === "elevated_privilege"}
  {@const irreversibleOther = p.reversibility === "irreversible" && !holdDestructive && !standingElsewhere}
  {@const reversibleDestructive = p.class === "destructive" && p.reversibility !== "irreversible"}
  {@const plainReversible = !holdDestructive && !standingElsewhere && !irreversibleOther && !reversibleDestructive}
  <Dialog.Root
    open={true}
    onOpenChange={(open) => {
      if (!open) deny(p);
    }}
  >
    <Dialog.Content>
      <div class="cd">
        {#if tone !== "neutral"}
          <span class="cd-edge tone-{tone}" aria-hidden="true"></span>
        {/if}

        <div class="cd-req">
          <Avatar>
            <AvatarFallback>{friendly(p.requester).charAt(0)}</AvatarFallback>
          </Avatar>
          <span class="cd-req-text">
            <span class="cd-req-name">{friendly(p.requester)}</span>
            <span class="cd-req-id">
              <ShieldCheck size={11} strokeWidth={2} aria-hidden="true" />{p.requester}
            </span>
          </span>
        </div>

        <h2 class="cd-title" class:big={p.tier === "high_stakes"}>
          Allow {friendly(p.requester)} to {p.summary}?
        </h2>

        {#if p.class === "external_send"}
          <div class="cd-field">
            <span class="cd-field-label">To</span>
            <span class="cd-field-val">{p.recipient ?? p.scope}</span>
          </div>
          {#if p.preview}
            <div class="cd-preview">
              <span class="cd-field-label">Preview</span>
              <pre class="cd-preview-body">{p.preview}</pre>
            </div>
          {/if}
        {:else if p.class === "destructive" && p.targets?.length}
          <ul class="cd-items">
            {#each p.targets as item}
              <li class="cd-item">
                <span class="cd-item-name">{item.name}</span>
                <span class="cd-item-size">{item.size}</span>
              </li>
            {/each}
          </ul>
          {#if p.total}
            <p class="cd-meta">{p.total} total</p>
          {/if}
        {:else if p.scope}
          <div class="cd-field">
            <span class="cd-field-label">{scopeLabel(p)}</span>
            <span class="cd-field-val">{p.scope}</span>
          </div>
        {/if}

        {#if p.triggeredExternally}
          <div class="cd-warn tone-caution">
            <AlertTriangle size={14} strokeWidth={2} aria-hidden="true" />
            Started by another app or document. Only continue if you expected this.
          </div>
        {/if}

        {#if standingElsewhere}
          <p class="cd-note">
            To let {friendly(p.requester)} do this on its own, allow it in App access.
          </p>
        {:else if reversibleDestructive}
          <p class="cd-note">You can undo this from the Trash.</p>
        {:else if plainReversible}
          <p class="cd-note">Reversible. Revoke anytime from App access.</p>
        {/if}

        <div class="cd-foot">
          {#if holdDestructive}
            <Button variant="outline" onclick={() => deny(p)}>Cancel</Button>
            <span class="cd-spacer"></span>
            <button
              type="button"
              class="cd-hold"
              class:holding
              onpointerdown={() => holdStart(p)}
              onpointerup={holdEnd}
              onpointerleave={holdEnd}
            >
              <span class="cd-hold-fill" aria-hidden="true"></span>
              <span class="cd-hold-label">
                <Trash2 size={16} strokeWidth={2} aria-hidden="true" />
                {holdLabel(p)}
              </span>
            </button>
          {:else if standingElsewhere}
            <Button variant="outline" onclick={() => deny(p)}>Deny</Button>
            <span class="cd-spacer"></span>
            {#if p.class === "external_send"}
              <Button onclick={() => allowOnce(p)}>
                <Send size={14} strokeWidth={2} aria-hidden="true" /> Send once
              </Button>
            {:else}
              <Button onclick={() => allowOnce(p)}>Allow once</Button>
            {/if}
          {:else if irreversibleOther}
            <Button variant="outline" onclick={() => deny(p)}>Deny</Button>
            <span class="cd-spacer"></span>
            <Button onclick={() => allowOnce(p)}>Allow once</Button>
          {:else}
            <Button variant="outline" onclick={() => deny(p)}>Deny</Button>
            <span class="cd-spacer"></span>
            <Button variant="ghost" onclick={() => allowRemember(p)}>Always allow</Button>
            <Button onclick={() => allowOnce(p)}>Allow once</Button>
          {/if}
        </div>
      </div>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .cd {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  /* The accent edge: a thin bar flush to the card's top, the ambient signal that
     the stakes are above routine. Absolute so it never disturbs the layout; the
     -1.25rem offsets counter the dialog's p-5 padding to reach the card edge. */
  .cd-edge {
    position: absolute;
    top: -1.25rem;
    left: -1.25rem;
    right: -1.25rem;
    height: 3px;
    border-radius: var(--radius-input) var(--radius-input) 0 0;
  }
  .cd-edge.tone-caution {
    background: var(--color-warning);
  }
  .cd-edge.tone-danger {
    height: 4px;
    background: var(--color-error);
  }

  /* WHO - the trust anchor. The monogram identifies the app; the id line carries
     the attested identity (shown == grant recipient) with the verified mark. */
  .cd-req {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .cd-req-text {
    display: flex;
    flex-direction: column;
    gap: 0.0625rem;
    min-width: 0;
  }
  .cd-req-name {
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--foreground);
  }
  .cd-req-id {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cd-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    line-height: 1.35;
    color: var(--foreground);
  }
  .cd-title.big {
    font-size: var(--text-lg);
  }

  /* A labelled field: the concrete scope/target/recipient, a quiet bordered box
     so the value reads as data, distinct from the prose. */
  .cd-field {
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
    padding: 0.5rem 0.625rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    border-radius: var(--radius-input);
  }
  .cd-field-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
  }
  .cd-field-val {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* External send - the content that would leave Arlen, verbatim, so "send once"
     is informed. Scrolls if long; never grows the dialog unbounded. */
  .cd-preview {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.625rem;
    border: 1px solid color-mix(in srgb, var(--color-warning) 30%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-warning) 6%, transparent);
  }
  .cd-preview-body {
    margin: 0;
    max-height: 6rem;
    overflow: auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    color: color-mix(in srgb, var(--foreground) 78%, transparent);
  }

  /* Destructive - the actual items lost, each named with its size. Names what the
     summary count hides. */
  .cd-items {
    margin: 0;
    padding: 0.125rem 0;
    list-style: none;
    display: flex;
    flex-direction: column;
  }
  .cd-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.25rem 0.125rem;
    font-size: var(--text-sm);
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .cd-item:last-child {
    border-bottom: none;
  }
  .cd-item-name {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cd-item-size {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .cd-meta {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .cd-warn {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .cd-warn.tone-caution {
    background: color-mix(in srgb, var(--color-warning) 12%, transparent);
    color: color-mix(in srgb, var(--color-warning) 92%, var(--foreground));
  }
  /* A quiet reassurance / pointer (reversible undo, where standing access lives)
     - not a wall, just a line. */
  .cd-note {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  .cd-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
  .cd-spacer {
    flex: 1;
  }

  /* The destructive hold-to-confirm: a filled bar sweeps left-to-right over the
     hold, the label rides on top. Error-toned, its own control (not a Button).
     The label names the target, so it cannot be answered from muscle memory. */
  .cd-hold {
    position: relative;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    max-width: 15rem;
    height: var(--height-control-prominent, 36px);
    padding: 0 0.625rem;
    border: 1px solid color-mix(in srgb, var(--color-error) 45%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    color: var(--color-error);
    font-size: var(--text-base);
    font-weight: 500;
    cursor: pointer;
    user-select: none;
  }
  .cd-hold-fill {
    position: absolute;
    inset: 0;
    width: 0;
    background: color-mix(in srgb, var(--color-error) 30%, transparent);
  }
  .cd-hold.holding .cd-hold-fill {
    width: 100%;
    transition: width 1.2s linear;
  }
  .cd-hold-label {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .cd-hold.holding .cd-hold-fill {
      transition: none;
    }
  }
</style>
