<script lang="ts">
  /// 15-second revert-on-timeout confirmation modal.
  ///
  /// Spec: `display-system.md` §A4. After Settings calls
  /// `applyConfig`, this modal opens with a countdown. If the user
  /// hits "Keep changes" within the window, we call
  /// `saveCurrent()` so the change persists. If the timer fires
  /// or the user picks "Revert", we call `revertConfig(snapshot)`
  /// to restore the pre-apply state.
  ///
  /// The modal also reacts to `lastApplyResult`: if the compositor
  /// rejected the apply (`failed` / `cancelled`), the modal closes
  /// itself with a brief error toast — no countdown was meaningful
  /// because the change never took effect.

  import {
    revertConfig,
    saveCurrent,
    lastApplyResult,
    type MonitorConfig,
  } from "$lib/stores/displays";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t } from "$lib/i18n/messages";
  import Rich from "@arlen/ui-kit/i18n/Rich.svelte";
  import { mark } from "@arlen/ui-kit/i18n/rich";

  interface Props {
    /** When true, the modal is visible and the countdown runs. */
    open: boolean;
    /** Pre-apply state, used as the revert target. */
    snapshot: MonitorConfig[];
    /** Active apply request id, used to filter incoming results. */
    requestId: string | null;
    onClose: () => void;
  }

  let { open, snapshot, requestId, onClose }: Props = $props();

  const COUNTDOWN_SECONDS = 15;
  let secondsLeft = $state(COUNTDOWN_SECONDS);
  let busy = $state(false);
  let error = $state<string | null>(null);
  /// Set when we have dispatched a revert and are waiting for the
  /// compositor's reply. Until the matching `displays:apply-result`
  /// arrives the modal stays open so the user does not lose the
  /// recovery affordance.
  let pendingRevertId = $state<string | null>(null);

  // Drive the countdown only while open. `setInterval` cleanup goes
  // through the effect's return value so closing or reopening the
  // modal does not leak a previous timer.
  $effect(() => {
    if (!open) {
      secondsLeft = COUNTDOWN_SECONDS;
      busy = false;
      error = null;
      pendingRevertId = null;
      return;
    }
    secondsLeft = COUNTDOWN_SECONDS;
    const handle = setInterval(() => {
      // Pause the countdown while a revert is mid-flight; otherwise
      // a slow compositor reply could trigger a second revert.
      if (pendingRevertId) return;
      secondsLeft = Math.max(0, secondsLeft - 1);
      if (secondsLeft === 0) {
        revert("timeout").catch(() => {});
      }
    }, 1000);
    return () => clearInterval(handle);
  });

  // Watch for apply-results that match our original apply request.
  // A failed / cancelled apply means the compositor refused — there
  // is nothing to revert because nothing took effect. Just close.
  $effect(() => {
    if (!open || !requestId) return;
    const r = $lastApplyResult;
    if (!r || r.requestId !== requestId) return;
    if (r.outcome === "failed" || r.outcome === "cancelled") {
      error = $t("s.revert.outcome", { outcome: r.outcome });
      // Slight delay so the user notices the message before the
      // modal closes itself.
      setTimeout(() => onClose(), 1500);
    }
  });

  // Watch for apply-results that match our revert request. Only
  // close on `succeeded`; on `failed` / `cancelled` we surface the
  // error and let the user retry, because the live config is still
  // the new (potentially unusable) one.
  $effect(() => {
    if (!open || !pendingRevertId) return;
    const r = $lastApplyResult;
    if (!r || r.requestId !== pendingRevertId) return;
    if (r.outcome === "succeeded") {
      pendingRevertId = null;
      busy = false;
      onClose();
    } else {
      error = $t("s.revert.failed");
      pendingRevertId = null;
      busy = false;
    }
  });

  async function keep() {
    if (busy) return;
    busy = true;
    try {
      await saveCurrent();
      onClose();
    } catch (err) {
      // A KEY, like the two sentences above it. This drew `String(err)` into the
      // alert body, so somebody whose display change would not stick met the
      // config writer's own words - in a dialogue that is counting down to undo
      // the change behind them. What they need is the consequence, and the
      // countdown is still running while they read it.
      console.warn("settings: keeping the display change failed", err);
      error = $t("s.revert.keepFailed");
      busy = false;
    }
  }

  async function revert(_reason: "timeout" | "cancel") {
    if (busy) return;
    busy = true;
    error = null;
    try {
      const id = await revertConfig(snapshot);
      // Hand off to the apply-result $effect; it will close the
      // modal on success or release the busy lock on failure.
      pendingRevertId = id;
    } catch (err) {
      // The same outcome the apply-result branch above words: the revert did not
      // go through and the live config is still the new one, so the sentence is
      // the same sentence. This one refused before the request was even sent, and
      // it used to answer with the writer's own words.
      console.warn("settings: the display revert did not start", err);
      error = $t("s.revert.failed");
      busy = false;
    }
  }
  /// Escape does what the countdown does. This modal is the last thing between
  /// a person and a display change they may not be able to see well enough to
  /// cancel with a pointer, and it had no keyboard way out at all - the two
  /// buttons were reachable only by tabbing into a dialog that never took focus.
  /// Escape reverts, which is both the safe direction and exactly what happens
  /// if they do nothing.
  /// The modal takes the focus when it opens and hands it back when it closes.
  /// It says `aria-modal="true"`, which is a claim that the rest of the page is
  /// inert - and until now nothing moved the focus in, so a keyboard was still
  /// tabbing through the page BEHIND the overlay and a screen reader was never
  /// told the dialog existed. Measured: `focusInside=false, active=BODY`.
  ///
  /// The card rather than a button: this one asks whether to keep a display
  /// change, and arming either answer under the return key is not a courtesy.
  let card = $state<HTMLElement | null>(null);
  let restoreTo: HTMLElement | null = null;

  $effect(() => {
    if (open) {
      restoreTo = document.activeElement as HTMLElement | null;
      card?.focus();
      return;
    }
    const back = restoreTo;
    restoreTo = null;
    back?.focus?.();
  });

  function onWindowKeydown(e: KeyboardEvent) {
    if (!open || busy || e.key !== "Escape") return;
    e.preventDefault();
    void revert("cancel");
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if open}
  <!-- Named after its own heading. A modal that says `role="dialog"` and nothing
       else is announced as an unnamed dialog, and this one is the last thing
       between a person and a display change they cannot see well enough to
       cancel. -->
  <div
    class="backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="revert-title"
    tabindex="-1"
    bind:this={card}
  >
    <div class="modal">
<!-- Its own key. It shared `s.revert.keep` with the button below, which
           takes a `{$seconds}` parameter - so the heading, called without one,
           rendered the placeholder verbatim: "Aenderungen behalten ({$seconds} s)".
           Two definitions of that id existed, in two catalogue files, and the
           merge order decided which one every caller got. -->
      <h2 id="revert-title">{$t("s.revert.title")}</h2>
      <p class="body">
        {#if pendingRevertId}
          {$t("s.revert.reverting")}
        {:else}
          <Rich text={$t("s.revert.countdown", { left: mark("left") })}>
            {#snippet left()}<strong>{$t("s.revert.seconds", { n: secondsLeft })}</strong>{/snippet}
          </Rich>
        {/if}
      </p>

      <progress class="bar" max={COUNTDOWN_SECONDS} value={secondsLeft}></progress>

      {#if error}
        <p class="error" role="alert">{error}</p>
      {/if}

      <div class="actions">
        <Button
          variant="outline"
          onclick={() => revert("cancel")}
          disabled={busy}
        >
          {$t("s.revert.now")}
        </Button>
        <Button onclick={keep} disabled={busy}>
          {$t("s.revert.keep", { seconds: secondsLeft })}
        </Button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* NO RING ON THE CARD ITSELF. The card takes focus when the dialog opens, so a
     person tabbing lands inside it and Escape reaches the handler - but WebKit
     draws its default ring on whatever is focused, and on a container that is a
     4.8px reddish outline around the whole dialog, which reads as an error. It
     matches `:focus-visible` too, so scoping to that does not help. The ring
     belongs on the controls inside; the dialog announces itself by being one. */
  .backdrop:focus {
    outline: none;
  }

  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .modal {
    width: 100%;
    max-width: 420px;
    padding: 20px;
    background: var(--color-bg-card);
    border: 1px solid color-mix(in srgb, var(--color-fg-app) 12%, transparent);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-app);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .body {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-app) 75%, transparent);
  }

  .body strong {
    color: var(--color-accent);
  }

  .bar {
    width: 100%;
    height: 4px;
    appearance: none;
    border: none;
    border-radius: 2px;
    background: color-mix(in srgb, var(--color-fg-app) 10%, transparent);
    overflow: hidden;
  }

  .bar::-webkit-progress-bar {
    background: color-mix(in srgb, var(--color-fg-app) 10%, transparent);
    border-radius: 2px;
  }

  .bar::-webkit-progress-value {
    background: var(--color-accent);
    border-radius: 2px;
    transition: width 200ms linear;
  }

  .bar::-moz-progress-bar {
    background: var(--color-accent);
    border-radius: 2px;
  }

  .error {
    margin: 0;
    padding: 8px 10px;
    background: color-mix(in srgb, var(--destructive) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    border-radius: var(--radius-chip);
    font-size: 0.8rem;
    color: var(--destructive);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
