/// Focus trap for modal overlays (dialogs, command palettes, popovers that own the
/// screen). Keeps Tab / Shift+Tab cycling within the node's focusable descendants so
/// keyboard + screen-reader users cannot fall out of the modal into the page behind
/// it, and returns focus to wherever it was before the overlay opened.
///
/// Escape is left to the consumer (it usually maps to the overlay's own close). The
/// shadcn `Dialog` primitive already traps focus; this is for CUSTOM overlays.

import type { Action } from "svelte/action";

/// Options for the {@link trapFocus} action.
export interface TrapFocusOptions {
  /// Restore focus to the element focused before the overlay opened, on destroy.
  /// Default true.
  returnFocus?: boolean;
  /// Focus the first focusable descendant on mount, unless the node already contains
  /// focus (so it never fights a consumer's own autofocus). Default true.
  autoFocus?: boolean;
}

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

function defaults(o: TrapFocusOptions | undefined): Required<TrapFocusOptions> {
  return { returnFocus: true, autoFocus: true, ...o };
}

/// A Svelte action: `use:trapFocus` on a modal container.
export const trapFocus: Action<HTMLElement, TrapFocusOptions | undefined> = (
  node: HTMLElement,
  options?: TrapFocusOptions,
) => {
  let opts = defaults(options);
  const previouslyFocused = document.activeElement as HTMLElement | null;

  function focusables(): HTMLElement[] {
    return [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (el) => el.offsetParent !== null || el === document.activeElement,
    );
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== "Tab") return;
    const list = focusables();
    if (list.length === 0) {
      e.preventDefault();
      node.focus();
      return;
    }
    const first = list[0];
    const last = list[list.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === node)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  node.addEventListener("keydown", onKeydown);
  if (opts.autoFocus) {
    queueMicrotask(() => {
      if (!node.contains(document.activeElement)) (focusables()[0] ?? node).focus();
    });
  }

  return {
    update(next: TrapFocusOptions | undefined) {
      opts = defaults(next);
    },
    destroy() {
      node.removeEventListener("keydown", onKeydown);
      if (opts.returnFocus && previouslyFocused && document.contains(previouslyFocused)) {
        previouslyFocused.focus();
      }
    },
  };
};
