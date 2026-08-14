/// Accessibility state for the greeter. a11y at login is mandatory and
/// cannot borrow the session's (greeter-onboarding-plan.md §2, the GDM
/// "Accessible Login" pattern): the greeter owns these toggles itself and
/// applies them to its own root immediately. The deeper screen-reader
/// (Orca/Newton) wiring is a flagged dependency; the markup is built for it.
import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

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

/// Whether the screen-reader toggle was OPERATED at this login.
///
/// Three-state on purpose, and the middle state is the one that matters:
/// `null` means nobody touched it, so the session must keep whatever that
/// user's own config says. The login screen's remembered default arriving on
/// screen is a fact about the door, not a statement about the person walking
/// through it - and somebody with the reader on inside their session must not
/// lose it because they walked past an untouched greeter.
///
/// `false` is as deliberate as `true`: reaching over and switching it off here
/// carries the same weight as switching it on.
export const screenReaderChoice = writable<boolean | null>(null);

/// Seed the toggles from what this login screen remembers.
///
/// Only the screen reader persists. The others (contrast, large text, on-screen
/// keyboard) are reachable without sight or a reader, so nothing is lost by
/// their starting fresh - and this deliberately does NOT mark the choice as
/// operated, because restoring a remembered default is not somebody making a
/// decision.
export async function loadRememberedA11y(): Promise<void> {
  try {
    // A bare boolean on purpose: a snake_case field on one side read as
    // camelCase on the other is `undefined`, which is falsy, so the login
    // screen would forget the setting with nothing to show for it.
    const remembered = await invoke<boolean>("greeter_a11y_get");
    if (remembered) {
      a11y.update((s) => ({ ...s, screenReader: true }));
    }
  } catch {
    // A login screen that will not draw because of one boolean is worse for
    // everybody than one that draws with the toggle off, and the toggle is
    // right there on screen.
  }
}

/// Flip one toggle.
///
/// The screen reader also gets remembered for the next boot and marked as
/// operated, so it travels forward with the session. Remembering happens on the
/// flip rather than on a successful login: somebody who switches the reader on
/// and then mistypes their password still needs it when they try again.
export function toggleA11y(key: keyof A11yState): void {
  a11y.update((s) => ({ ...s, [key]: !s[key] }));
  if (key === "screenReader") {
    const on = get(a11y).screenReader;
    screenReaderChoice.set(on);
    void invoke("greeter_a11y_set", { screenReader: on }).catch(() => {
      // It still applies to this login; it just will not be there next boot.
    });
  }
}
