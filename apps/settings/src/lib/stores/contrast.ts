/// Whether the active appearance is legible, measured rather than assumed.
///
/// `theme_contrast_report` audits every key foreground-over-background pair of
/// the resolved theme against the WCAG 2.2 AA floor (the legal one, EN 301 549)
/// and the APCA bronze floor (the perceptual one). The compute lives in
/// `sdk/theme` and the command has been registered with nobody asking, so a
/// person could pick a theme that fails both and the accessibility page - the
/// one surface whose job is exactly this - would not say a word.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One audited pair (the Rust `ContrastRole`).
export interface ContrastRole {
  /// Role names, e.g. `fg.primary on bg.app`. Identifiers from the theme
  /// schema, so they pass through as they are: a person reading this row is
  /// looking for which pair to change, and the theme file calls it this.
  pair: string;
  wcag: number;
  apca: number;
  /// `body` or `large` - which floor this pair is held to.
  usage: string;
  wcagPass: boolean;
  apcaPass: boolean;
}

/// What the audit said. Null while it is being read.
const _report = writable<ContrastRole[] | null>(null);
export const contrastReport: Readable<ContrastRole[] | null> = {
  subscribe: _report.subscribe,
};

/// True when the theme could not be resolved, so nothing was measured.
///
/// Distinct from an empty report, which would mean the audit ran and found no
/// pair to check. Neither is "your theme is fine", and the page must not draw
/// them as if they were.
export const contrastUnavailable = writable(false);

/// Run the audit against the active appearance.
export async function loadContrast(): Promise<void> {
  _report.set(null);
  contrastUnavailable.set(false);
  try {
    _report.set(await invoke<ContrastRole[]>("theme_contrast_report"));
  } catch (e) {
    console.warn("[settings] contrast report failed:", e);
    contrastUnavailable.set(true);
  }
}

/// The pairs that miss a floor - the only ones worth listing.
///
/// A page that printed all twenty rows with their numbers would be a report
/// nobody reads; what a person needs is the ones to fix, and silence when there
/// are none.
export function failing(report: ContrastRole[]): ContrastRole[] {
  return report.filter((r) => !r.wcagPass || !r.apcaPass);
}
