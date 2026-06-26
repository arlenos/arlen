//! Terminal font-size zoom (terminal-ui-plan.md §5b). The Ctrl +/-/0 shortcuts
//! apply a transient delta over the persisted base size (the base is owned by the
//! terminal config; the Settings UI sets it, the daemon clamps + persists it).
//! Pure helpers so the step and key classification are unit-tested.

/// The on-screen font-size bounds. Mirrors the daemon's `clamp_font_size`
/// (apps/terminal/src-tauri) so a zoomed size is always one the config accepts.
export const FONT_SIZE_MIN = 6;
export const FONT_SIZE_MAX = 72;

/// Clamp a font size into the readable range (mirrors the backend clamp). A
/// non-finite value is returned to the caller's fallback unchanged - the base is
/// always finite, this only guards arithmetic.
export function clampFontSize(px: number): number {
  if (!Number.isFinite(px)) return px;
  return Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, px));
}

/// One zoom step from `current`, by 1px, clamped. `in` enlarges, `out` shrinks.
/// Rounds first so a fractional starting size lands on whole pixels.
export function zoomStep(current: number, dir: "in" | "out"): number {
  const next = Math.round(current) + (dir === "in" ? 1 : -1);
  return clampFontSize(next);
}

/// A keyboard zoom action, or `null` when the event is not a zoom shortcut.
export type ZoomAction = "in" | "out" | "reset";

/// Classify a keydown as a zoom action. The universal terminal convention:
/// Ctrl with `=`/`+` zooms in, with `-`/`_` out, with `0` resets to the base.
/// Shift is tolerated (Ctrl+Shift+`=` is `+` on many layouts); Alt/Meta exclude,
/// so window/WM chords are never stolen.
export function matchZoom(e: {
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  key: string;
}): ZoomAction | null {
  if (!e.ctrlKey || e.altKey || e.metaKey) return null;
  switch (e.key) {
    case "=":
    case "+":
      return "in";
    case "-":
    case "_":
      return "out";
    case "0":
      return "reset";
    default:
      return null;
  }
}
