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
  import { Button } from "$lib/components/ui/button";

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
      error = `Compositor ${r.outcome === "failed" ? "rejected" : "cancelled"} the change.`;
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
      error = `Revert ${r.outcome === "failed" ? "rejected" : "cancelled"} by compositor — try again.`;
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
      error = String(err);
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
      error = String(err);
      busy = false;
    }
  }
</script>

{#if open}
  <div class="backdrop" role="dialog" aria-modal="true">
    <div class="modal">
      <h2>Keep these display changes?</h2>
      <p class="body">
        {#if pendingRevertId}
          Reverting…
        {:else}
          The new layout has been applied. If you can read this clearly,
          confirm to keep it. Otherwise we'll roll back automatically in
          <strong>{secondsLeft}s</strong>.
        {/if}
      </p>

      <progress class="bar" max={COUNTDOWN_SECONDS} value={secondsLeft}></progress>

      {#if error}
        <p class="error">{error}</p>
      {/if}

      <div class="actions">
        <Button
          variant="outline"
          onclick={() => revert("cancel")}
          disabled={busy}
        >
          Revert now
        </Button>
        <Button onclick={keep} disabled={busy}>
          Keep changes ({secondsLeft}s)
        </Button>
      </div>
    </div>
  </div>
{/if}

<style>
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
    font-size: 1rem;
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
