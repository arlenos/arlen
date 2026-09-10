<script lang="ts">
  /// The modal shell: a centered card over a dimming backdrop, the one frame
  /// every Arlen dialog sits in. Escape and a backdrop click close it (both
  /// suppressed while `dismissable` is false, e.g. mid-operation). The caller
  /// owns the content - header, body, actions - and its padding; this owns only
  /// the frame, the modal radius and the dismiss behaviour, so confirm prompts,
  /// the conflict dialog, batch-rename and About all read as one surface.
  import type { Snippet } from "svelte";

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

  let card: HTMLDivElement | null = $state(null);

  function onBackdropClick(e: MouseEvent): void {
    // Only the backdrop itself, never a click bubbled from the card.
    if (e.target === e.currentTarget && dismissable) onClose();
  }

  //: What a person can tab to, in the order they would reach it. Visible only:
  //: a control inside a collapsed section is in the DOM and not on the screen,
  //: and tabbing to it would move the cursor somewhere nobody can see.
  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]),' +
    ' select:not([disabled]), textarea:not([disabled]),' +
    ' [tabindex]:not([tabindex="-1"])';

  function focusablesIn(root: HTMLElement): HTMLElement[] {
    return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (e) => e.getClientRects().length > 0,
    );
  }

  // WHERE THE KEYBOARD GOES WHEN THIS OPENS, and it used to go nowhere.
  //
  // This card has said `aria-modal="true"` since it was written, which tells a
  // screen reader the rest of the page is not there - and it carried
  // `tabindex="-1"` so it COULD be focused, and nothing ever focused it. Driven
  // through `escape-dismisses.js` on 11 September, two of the three dialogs in
  // Settings answered `focusBefore=body`: the dialog was open, modal, and the
  // reader's cursor was still out in a region their own screen reader had just
  // been told to ignore. A sighted mouse user never meets it, which is why it
  // survived a page that has been looked at a dozen times.
  //
  // Three things, and they are one property: focus moves IN when it opens, Tab
  // stays inside while it is open, and focus goes BACK to whatever opened it
  // when it closes. The last one is why the opener is captured here rather than
  // read at close time - by then the element is gone from `document.activeElement`.
  $effect(() => {
    if (!open) return;
    const opener = document.activeElement as HTMLElement | null;
    return () => {
      // Only if it is still on the page: a dialog that removed its own trigger
      // (an uninstall, a delete) would otherwise throw focus at a detached node
      // and land it on `body` anyway, with a exception on the way.
      if (opener && opener.isConnected && typeof opener.focus === "function") {
        opener.focus({ preventScroll: true });
      }
    };
  });

  $effect(() => {
    if (!open || !card) return;
    const inside = focusablesIn(card);
    // The card itself when it holds no control - a dialog that is only words
    // still has to be where the keyboard is, or the next Tab starts at the top
    // of the document behind it.
    (inside[0] ?? card).focus({ preventScroll: true });

    function onKeydown(e: KeyboardEvent): void {
      if (e.key === "Escape" && dismissable) {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== "Tab" || !card) return;
      const items = focusablesIn(card);
      if (items.length === 0) {
        e.preventDefault();
        card.focus({ preventScroll: true });
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      const out = !card.contains(active);
      if (e.shiftKey && (active === first || out)) {
        e.preventDefault();
        last.focus({ preventScroll: true });
      } else if (!e.shiftKey && (active === last || out)) {
        e.preventDefault();
        first.focus({ preventScroll: true });
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
      bind:this={card}
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
