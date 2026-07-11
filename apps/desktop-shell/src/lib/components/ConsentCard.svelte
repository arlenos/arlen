<script lang="ts">
  /// The presentational shell every permission request wears
  /// (system-dialog-plan.md). It is transport-free: it renders the common
  /// chrome - the attested requester as a trust anchor, the stakes-scaled accent
  /// edge, the plain-language ask - and takes the request-specific body and the
  /// decision affordances as snippets. The three permission surfaces
  /// (`ConsentDialog` off the broker, the AI-authorization prompt, the bluetooth
  /// pairing request) all render through this, so the whole cluster speaks one
  /// visual language while each keeps its own transport.
  import type { Snippet } from "svelte";
  import { Avatar, AvatarFallback } from "@arlen/ui-kit/components/ui/avatar";
  import { ShieldCheck } from "lucide-svelte";

  let {
    requesterName,
    requesterId,
    tone = "neutral",
    attested = true,
    title,
    big = false,
    body,
    footer,
  }: {
    /// The friendly name of who is asking (an app, or a device).
    requesterName: string;
    /// The exact attested identity beneath the name (an app id, a device
    /// address). The shown identity IS what a grant would be recorded against.
    requesterId: string;
    /// The single semantic accent: none, amber caution, red danger.
    tone?: "neutral" | "caution" | "danger";
    /// Whether the identity is system-attested (apps yes → the verified mark;
    /// a bluetooth device is not, so it passes false).
    attested?: boolean;
    /// The plain-language ask, already phrased (never the raw resource).
    title: string;
    /// A heavier title for high-stakes requests.
    big?: boolean;
    /// The request-specific body: scope, preview, targets, a passkey, a note.
    body?: Snippet;
    /// The decision affordances: the deny/allow/confirm buttons for this class.
    footer?: Snippet;
  } = $props();
</script>

<div class="cd">
  {#if tone !== "neutral"}
    <span class="cd-edge tone-{tone}" aria-hidden="true"></span>
  {/if}

  <div class="cd-req">
    <Avatar>
      <AvatarFallback>{requesterName.charAt(0)}</AvatarFallback>
    </Avatar>
    <span class="cd-req-text">
      <span class="cd-req-name">{requesterName}</span>
      <span class="cd-req-id">
        {#if attested}<ShieldCheck size={11} strokeWidth={2} aria-hidden="true" />{/if}{requesterId}
      </span>
    </span>
  </div>

  <h2 class="cd-title" class:big>{title}</h2>

  {@render body?.()}

  <div class="cd-foot">
    {@render footer?.()}
  </div>
</div>

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

  /* WHO - the trust anchor. The monogram identifies the requester; the id line
     carries the attested identity (shown == grant recipient) with the mark. */
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

  .cd-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
</style>
