<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import { clearSurface } from "$lib/surface-clear";
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
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { AlertTriangle, Send, Trash2 } from "lucide-svelte";
  import ConsentCard from "$lib/components/ConsentCard.svelte";
  import { current, resolve, pollConsent, type PendingView } from "$lib/stores/consent";

  /// How often to ask the broker for a pending request. A Confirm blocks the
  /// assistant until answered, so the user should see it promptly; the fetch is a
  /// one-shot round trip that returns None on an empty queue, so a second is
  /// cheap. Replace with the broker signal when the control protocol grows one.
  const POLL_MS = 1000;

  /// Backstop for dropping the card if its close animation never reports ending
  /// (no animation configured, or the event is missed). Comfortably longer than
  /// the fade, because ending the wait EARLY is the failure this exists to avoid.
  const CLOSE_FALLBACK_MS = 1000;

  // The broker has no push channel, so the dialog has to ask. Fetching only once
  // on mount made the whole surface inert in practice: a Confirm arrives when the
  // assistant proposes something, which is always AFTER the shell started, so the
  // single startup fetch always saw an empty queue and nothing was ever shown to
  // the user again. The store's doc names a "broker-signal listener that drives the
  // fetch" as the intended design; that needs a subscribe op the broker's control
  // protocol does not have yet, and polling is the honest stand-in until it does -
  // a fetch on an empty queue is a cheap one-shot round trip that returns None, and
  // when the signal lands it replaces this interval without the dialog changing.
  onMount(() => {
    void pollConsent();
    const timer = setInterval(() => void pollConsent(), POLL_MS);
    return () => clearInterval(timer);
  });

  // The shell's full-surface input region follows the CARD, not the request. The
  // window's default region is the top bar only, so while the card is up the
  // region must cover the surface or a click on Allow falls through to the
  // desktop; the keyboard grab rides along for Escape-to-deny.
  //
  // It matters that this releases LATE. Driven off the request (which is how it
  // used to work, from the store) it fired the moment the answer landed, changing
  // the layer surface's input shape and keyboard mode while the card was still
  // fading - and on the image the fade stops delivering frames at about that
  // point, leaving the card frozen half-drawn. Whether or not that is the cause,
  // holding the region until the card is really gone is the correct order on its
  // own terms: the surface stays interactive for exactly as long as something
  // interactive is on it.
  let regionShown = false;
  $effect(() => {
    const shown = view !== null;
    if (shown === regionShown) return;
    regionShown = shown;
    void invoke("set_consent_input_region", { active: shown }).catch(() => {});
  });

  /// One line into the journal. The shell has no devtools on the image, so this is
  /// the only way a frontend fact becomes visible to a boot log.
  const say = (msg: string, level = "info") =>
    void invoke("frontend_log", { level, msg }).catch(() => {});

  // Never take away a node that is still painting.
  //
  // On the image, removing the card outright leaves its pixels on the screen: the
  // DOM goes clean (the check below reports zero cards) and the card is still
  // there. Whatever the surface does with the damage for an element that ceases to
  // exist, it does not repaint what it vacated - while ordinary changes in the same
  // area do arrive, which is why the fade is visible at all and why Quick Settings
  // opening beside it paints perfectly.
  //
  // So the close is driven through the primitive, and the node is made to PAINT
  // itself empty before it is taken away. A first attempt dropped it 250ms after
  // the answer, past the fade's own 167ms, and the card still froze half-faded:
  // the animation had finished logically, but on a software-rendered VM the last
  // frames of it never reached the screen, and once the node was gone nothing
  // redrew that area again. Waiting for `animationend` alone has the same hole,
  // only narrower.
  //
  // Hiding it explicitly closes the hole from the other side. Whatever the fade
  // managed to paint, the card is then painted once more as nothing, and only a
  // node that is already invisible on screen is removed - so a missing repaint on
  // removal costs nothing.
  let view = $state<PendingView | null>(null);
  // Clear the surface when the card changes. Note what this is NOT for: the
  // after-answer residue it was first written for does not exist. Re-driven on
  // the image on 10 August with the pointer masked out of the check, answering
  // 'Allow once' returns the card's area to desktop. The earlier reading that
  // said otherwise was the mouse pointer sitting where the click had just landed.
  //
  // What is left is the case this shape is actually measured on elsewhere: a card
  // that changes while the surface stays mapped, where a second queued request
  // replacing a taller first one vacates the region the taller one occupied. That
  // is the waypointer's shrink case, which IS measured, arriving on this surface.
  // It has not been reproduced here - it needs two queued requests - so this is a
  // known-mechanism guard rather than a fix for an observed break.
  //
  // The target is `document.body` because the whole card lives inside
  // `{#if view}`, so every element it owns is gone at the moment stale pixels
  // would need reaching. See `$lib/surface-clear` for what the two steps do.
  $effect(() => {
    view;
    if (typeof document !== "undefined") clearSurface(document.body);
  });
  $effect(() => {
    const pending = $current;
    if (pending) {
      view = pending;
      return;
    }
    if (view === null) return;
    let dropped = false;
    const drop = () => {
      if (dropped) return;
      dropped = true;
      const card = document.querySelector<HTMLElement>(".arlen-consent-card");
      const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
      for (const el of [card, overlay]) {
        if (el) el.style.visibility = "hidden";
      }
      // Two frames, so the hidden state is painted and reaches the screen before
      // the node it belongs to stops existing. A window that is not animating can
      // have its frame callbacks throttled indefinitely, which would strand the
      // node mounted for good, so a timer finishes the job if the frames do not.
      let done = false;
      const finish = () => {
        if (done) return;
        done = true;
        view = null;
        // Nothing here can repair the pixels the card leaves behind, and that is
        // measured rather than assumed: a forced whole-page composite did not
        // clear them, and neither did a full `location.reload()` given fifteen
        // seconds to finish - the bar came back repainted and the card stayed.
        // A red band painted from the GTK draw handler never reached the screen
        // either. So the stale region is below both the page and the window, in
        // how the webview's surface is presented or in the compositor, and any
        // further attempt from this component would be superstition.
      };
      requestAnimationFrame(() => requestAnimationFrame(finish));
      setTimeout(finish, 200);
    };
    const card = document.querySelector(".arlen-consent-card");
    card?.addEventListener("animationend", drop, { once: true });
    const timer = setTimeout(drop, CLOSE_FALLBACK_MS);
    return () => {
      card?.removeEventListener("animationend", drop);
      clearTimeout(timer);
    };
  });

  // A dialog the user cannot dismiss is the worst failure this surface has: it
  // covers the desktop and every later request queues behind it, so answering one
  // is checked rather than assumed. The card renders into a portal on
  // document.body, outside this component, so nothing in the subtree can see it.
  let lastId: number | null = null;
  $effect(() => {
    const pending = $current;
    const cleared = lastId !== null && pending === null;
    const answeredId = lastId;
    lastId = pending?.id ?? null;
    if (!cleared) return;
    // Report BOTH outcomes, and report that the check started at all. A check that
    // only speaks when it finds something wrong is worthless the moment it stays
    // quiet: silence then means either a clean teardown or a check that never ran,
    // and those want opposite fixes. The first boot with this in place said
    // nothing, which is exactly that dead end.
    say(`consent: request ${answeredId} answered, checking that the card came down`);
    // After a frame, so a teardown that merely runs late is not called stuck.
    setTimeout(() => {
      const left = document.querySelectorAll(".arlen-consent-card").length;
      const others = document.querySelectorAll('[data-slot="dialog-content"]').length;
      if (left === 0) {
        say(`consent: card for ${answeredId} is out of the DOM (${others} dialog node(s) remain)`);
        return;
      }
      say(
        `consent: request ${answeredId} answered but ${left} card(s) still in the DOM `
          + `(${others} dialog node(s) total); the dialog is stuck on screen`,
        "error",
      );
    }, 250);
  });

  // A pending request must always be deniable by Escape. `open` is controlled by
  // the store, and a controlled dialog does not reliably fire the primitive's
  // escape-close, so deny explicitly here.
  function onWindowKeydown(e: KeyboardEvent): void {
    if (e.key !== "Escape") return;
    const p = get(current);
    if (p) deny(p);
  }

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
    if (p.class === "external_send") return $t("sh.consent.to");
    if (p.class === "destructive") return $t("sh.consent.target");
    if (p.class === "network_access") return $t("sh.consent.host");
    return $t("sh.consent.scope");
  }
  // Habituation defeat: with a single target the confirm names it, so the button
  // reads differently each time and cannot be answered from muscle memory. With
  // several, the list above already names them - the button stays plain rather
  // than repeat the count.
  function holdLabel(p: PendingView): string {
    if (p.targets && p.targets.length === 1)
      return $t("sh.consent.holdToDeleteNamed", { name: p.targets[0].name });
    return $t("sh.consent.holdToDelete");
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

<svelte:window onkeydown={onWindowKeydown} />

{#if view}
  {@const p = view}
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
  {#snippet body()}
    {#if p.class === "external_send"}
      <div class="cd-field">
        <span class="cd-field-label">{$t("sh.consent.to")}</span>
        <span class="cd-field-val">{p.recipient ?? p.scope}</span>
      </div>
      {#if p.preview}
        <div class="cd-preview">
          <span class="cd-field-label">{$t("sh.consent.preview")}</span>
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
        <p class="cd-meta">{$t("sh.consent.total", { count: p.total })}</p>
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
        {$t("sh.consent.external")}
      </div>
    {/if}

    {#if standingElsewhere}
      <p class="cd-note">
        {$t("sh.consent.standing", { app: friendly(p.requester) })}
      </p>
    {:else if reversibleDestructive}
      <p class="cd-note">{$t("sh.consent.undoTrash")}</p>
    {:else if plainReversible}
      <p class="cd-note">{$t("sh.consent.reversible")}</p>
    {/if}
  {/snippet}

  {#snippet footer()}
    {#if holdDestructive}
      <Button variant="outline" onclick={() => deny(p)}>{$t("sh.consent.cancel")}</Button>
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
      <Button variant="outline" onclick={() => deny(p)}>{$t("sh.consent.deny")}</Button>
      <span class="cd-spacer"></span>
      {#if p.class === "external_send"}
        <Button onclick={() => allowOnce(p)}>
          <Send size={14} strokeWidth={2} aria-hidden="true" /> {$t("sh.consent.sendOnce")}
        </Button>
      {:else}
        <Button onclick={() => allowOnce(p)}>{$t("sh.consent.allowOnce")}</Button>
      {/if}
    {:else if irreversibleOther}
      <Button variant="outline" onclick={() => deny(p)}>{$t("sh.consent.deny")}</Button>
      <span class="cd-spacer"></span>
      <Button onclick={() => allowOnce(p)}>{$t("sh.consent.allowOnce")}</Button>
    {:else}
      <Button variant="outline" onclick={() => deny(p)}>{$t("sh.consent.deny")}</Button>
      <span class="cd-spacer"></span>
      <Button variant="ghost" onclick={() => allowRemember(p)}>{$t("sh.consent.alwaysAllow")}</Button>
      <Button onclick={() => allowOnce(p)}>{$t("sh.consent.allowOnce")}</Button>
    {/if}
  {/snippet}

  <Dialog.Root
    open={$current !== null}
    onOpenChange={(open) => {
      // Only a real dismissal by the user is a denial. Our own close - the store
      // already answered - drives `open` false too, and denying then would send a
      // second verdict for a request that is gone.
      if (!open && $current !== null) deny(p);
    }}
  >
    <!-- The marker the teardown self-check below looks for. The card renders into
         a portal on document.body, outside this component's subtree, so there is
         no other way to ask whether it actually went away. -->
    <Dialog.Content class="arlen-consent-card">
      <ConsentCard
        requesterName={friendly(p.requester)}
        requesterId={p.requester}
        {tone}
        title={`Allow ${friendly(p.requester)} to ${p.summary}?`}
        big={p.tier === "high_stakes"}
        {body}
        {footer}
      />
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
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
