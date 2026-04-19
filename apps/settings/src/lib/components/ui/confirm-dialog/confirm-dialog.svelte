<script lang="ts">
  /// Generic confirm dialog.
  ///
  /// Canonical source lives in `sdk/ui-kit`; consuming apps keep file
  /// copies under their own `src/lib/components/ui/confirm-dialog/`
  /// (Tailwind scope hashing breaks across symlinked components — sync
  /// by copying when this file changes).
  ///
  /// Controlled from the parent via `open`. Parent supplies title,
  /// message, and an async `onConfirm` callback; the dialog awaits the
  /// callback so the confirm button can show a brief "Working…" state
  /// for slow operations. Escape and backdrop click both cancel.

  import { Button } from "$lib/components/ui/button";

  type Variant = "default" | "destructive";

  type Props = {
    open: boolean;
    title: string;
    message: string;
    /// Button label on the confirm side. Defaults to "Confirm".
    confirmLabel?: string;
    /// Visual intent for the confirm button. `destructive` styles it
    /// in the error colour to signal irreversibility.
    variant?: Variant;
    onConfirm: () => void | Promise<void>;
    onCancel: () => void;
  };

  let {
    open,
    title,
    message,
    confirmLabel = "Confirm",
    variant = "default",
    onConfirm,
    onCancel,
  }: Props = $props();

  let busy = $state(false);

  async function handleConfirm(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      await onConfirm();
    } finally {
      busy = false;
    }
  }

  function handleBackdropClick(e: MouseEvent): void {
    // Only cancel if the click was on the backdrop, not bubbled from
    // inside the dialog body.
    if (e.target === e.currentTarget && !busy) {
      onCancel();
    }
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape" && !busy) {
      e.preventDefault();
      onCancel();
    }
  }

  $effect(() => {
    if (open) {
      busy = false;
      window.addEventListener("keydown", handleKeydown, { capture: true });
      return () => {
        window.removeEventListener("keydown", handleKeydown, {
          capture: true,
        });
      };
    }
  });
</script>

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
    aria-labelledby="confirm-dialog-title"
    tabindex="-1"
    onclick={handleBackdropClick}
    onkeydown={handleKeydown}
  >
    <div
      class="w-full max-w-md rounded-[var(--radius)] border border-border bg-card p-6 shadow-lg"
    >
      <h2
        id="confirm-dialog-title"
        class="mb-2 text-base font-semibold text-foreground"
      >
        {title}
      </h2>
      <p class="mb-6 text-sm text-muted-foreground">{message}</p>
      <div class="flex justify-end gap-2">
        <Button variant="ghost" onclick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button
          variant={variant === "destructive" ? "destructive" : "default"}
          onclick={handleConfirm}
          disabled={busy}
        >
          {busy ? "Working…" : confirmLabel}
        </Button>
      </div>
    </div>
  </div>
{/if}
