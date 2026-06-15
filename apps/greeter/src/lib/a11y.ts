/// Accessibility state for the greeter. a11y at login is mandatory and
/// cannot borrow the session's (greeter-onboarding-plan.md §2, the GDM
/// "Accessible Login" pattern): the greeter owns these toggles itself and
/// applies them to its own root immediately. The deeper screen-reader
/// (Orca/Newton) wiring is a flagged dependency; the markup is built for it.
import { writable } from "svelte/store";

/// The live accessibility options. All default off; each takes effect the
/// moment it flips, with no session and no restart.
export interface A11yState {
  /// Maximum-contrast palette: the floating layout swaps to an opaque,
  /// strong-bordered surface (see app.css [data-contrast="high"]).
  highContrast: boolean;
  /// Scale the greeter's type up for low vision.
  largeText: boolean;
  /// Show the on-screen keyboard for password entry without hardware keys.
  onScreenKeyboard: boolean;
  /// Surface the screen-reader hint (a real reader is a deeper dependency).
  screenReader: boolean;
}

const initial: A11yState = {
  highContrast: false,
  largeText: false,
  onScreenKeyboard: false,
  screenReader: false,
};

export const a11y = writable<A11yState>(initial);

/// Reflect the contrast and text-scale options onto the document root so
/// the CSS variables (`data-contrast`, `--greeter-scale`) take effect.
/// Called from the layout whenever the state changes.
export function applyA11y(state: A11yState): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (state.highContrast) root.dataset.contrast = "high";
  else delete root.dataset.contrast;
  root.style.setProperty("--greeter-scale", state.largeText ? "1.25" : "1");
}

/// Flip one toggle.
export function toggleA11y(key: keyof A11yState): void {
  a11y.update((s) => ({ ...s, [key]: !s[key] }));
}
