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

  import { Button } from "../button";
  import Dialog from "../dialog/dialog.svelte";

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

  // Reset the confirm button's working state each time the dialog opens; the
  // shell (`Dialog`) owns the backdrop, Escape and backdrop-click dismissal.
  $effect(() => {
    if (open) busy = false;
  });
</script>

<Dialog
  {open}
  onClose={onCancel}
  dismissable={!busy}
  labelledby="confirm-dialog-title"
  size="md"
>
  <div class="p-6">
    <h2
      id="confirm-dialog-title"
      class="mb-2 text-base font-semibold text-foreground"
    >
      {title}
    </h2>
    <p class="mb-6 text-sm text-muted-foreground">{message}</p>
    <div class="flex justify-end gap-2">
      <Button variant="ghost" onclick={onCancel} disabled={busy}>Cancel</Button>
      <Button
        variant={variant === "destructive" ? "destructive" : "default"}
        onclick={handleConfirm}
        disabled={busy}
      >
        {busy ? "Working…" : confirmLabel}
      </Button>
    </div>
  </div>
</Dialog>
