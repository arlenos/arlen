<script lang="ts">
  /// The modal shell: a centered card over a dimming backdrop, the one frame
  /// every Arlen dialog sits in. Escape and a backdrop click close it (both
  /// suppressed while `dismissable` is false, e.g. mid-operation). The caller
  /// owns the content - header, body, actions - and its padding; this owns only
  /// the frame, the modal radius and the dismiss behaviour, so confirm prompts,
  /// the conflict dialog, batch-rename and About all read as one surface.
  import type { Snippet } from "svelte";
  import { trapFocus } from "../../../keyboard/index.js";

  type Props = {
    /// Whether the dialog is mounted.
    open: boolean;
    /// Backdrop click and Escape call this (while dismissable).
    onClose: () => void;
    /// An accessible label when no visible titled element can be pointed at.
    ariaLabel?: string;
    /// The id of the visible title element, preferred over `ariaLabel`.
    labelledby?: string;
    /// The card's max-width tier.
    size?: "sm" | "md" | "lg";
    /// When false, Escape and backdrop click do not close (a running op).
    dismissable?: boolean;
    /// The dialog content, laid out by the caller.
    children: Snippet;
  };

  let {
    open,
    onClose,
    ariaLabel,
    labelledby,
    size = "md",
    dismissable = true,
    children,
  }: Props = $props();

  function onBackdropClick(e: MouseEvent): void {
    // Only the backdrop itself, never a click bubbled from the card.
    if (e.target === e.currentTarget && dismissable) onClose();
  }

  // WHERE THE KEYBOARD GOES, and it used to go nowhere. This card has said
  // `aria-modal="true"` since it was written, which tells a screen reader the
  // rest of the page is not there - and it carried `tabindex="-1"` so it COULD
  // be focused, and nothing ever focused it. Driven through
  // `escape-dismisses.js` on 11 September, two of the three dialogs in Settings
  // answered `focusBefore=body`: the dialog was open, modal, and the reader's
  // cursor was still out in a region their own screen reader had just been told
  // to ignore.
  //
  // `trapFocus` is the kit's own action and does all three halves of the
  // property - focus in on mount, Tab kept inside, focus returned to whatever
  // opened it on destroy. I wrote those three by hand here first and only then
  // found this; the command palettes in the shell had been using it all along.
  $effect(() => {
    if (!open) return;
    function onKeydown(e: KeyboardEvent): void {
      if (e.key === "Escape" && dismissable) {
        e.preventDefault();
        onClose();
      }
    }
    // Capture so the dialog closes even when focus sits in a field inside it.
    window.addEventListener("keydown", onKeydown, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKeydown, { capture: true });
  });
</script>

{#if open}
  <div class="dialog-backdrop" role="presentation" onclick={onBackdropClick}>
    <div
      use:trapFocus
      class="dialog-card dialog-{size}"
      role="dialog"
      aria-modal="true"
      aria-label={ariaLabel}
      aria-labelledby={labelledby}
      tabindex="-1"
    >
      {@render children()}
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: var(--color-bg-overlay, #00000080);
    -webkit-backdrop-filter: blur(2px);
    backdrop-filter: blur(2px);
  }
  .dialog-card {
    width: 100%;
    border: 1px solid
      var(--border, color-mix(in srgb, var(--foreground) 12%, transparent));
    border-radius: var(--radius-modal, 16px);
    background: var(--card, var(--color-bg-card));
    box-shadow: var(--shadow-lg, 0 12px 32px rgb(0 0 0 / 0.35));
    /* Inset children hugging the card can read this for concentric corners. */
    --container-radius: var(--radius-modal, 16px);
  }
  .dialog-sm {
    max-width: 360px;
  }
  .dialog-md {
    max-width: 460px;
  }
  .dialog-lg {
    max-width: 760px;
  }
</style>
