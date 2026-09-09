/// The cross-page "my customisations" summary for the Appearance landing: how
/// many fields the person has overridden on top of the active theme, broken down
/// by page, and a reset-all.
///
/// **Both halves used to be about the session rather than the machine.** The
/// counts came from suite stores that are only filled by visiting the page that
/// owns them, so arriving here fresh said "you have not changed anything" over a
/// `theme.toml` holding a dozen overrides, and the number only became true after
/// walking every page. And the reset set those stores to empty and wrote nothing,
/// so the summary went to zero while the machine kept everything. `loadSummary`
/// and `resetAll` below are the two fixes, and they belong together: a truthful
/// count in front of a reset that does nothing is still a surface saying
/// something the machine never learned.

import { derived, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";
import { overrides as colours, loadColorOverrides } from "./themeColors";
import { overrides as typography, load as loadTypography } from "./themeTypography";
import { overrides as motionDepth, loadMd } from "./themeMotionDepth";
import { overrides as system, loadSys } from "./themeSystem";
import {
  disabled as tkDisabled,
  accentOverrides as tkAccent,
  loadEnabled,
  loadOverrides as loadToolkitAccents,
} from "./themeToolkits";

/// One page's override count + where to reach it.
export interface OverridePage {
  key: string;
  labelKey: string;
  href: string;
  count: number;
}

/// The live summary: the per-page counts and the total.
///
/// **Geometry is not here, and its absence is the honest reading rather than an
/// omission.** That page writes nothing - its own header says so - so its store
/// holds what somebody dragged in this window and the machine holds none of it.
/// A "geometry: 3 changes" row would send a person to review changes that were
/// never saved, and the reset below could not take them away because they are
/// not anywhere. The row comes back with the page's backend.
export const overrideSummary = derived(
  [colours, typography, motionDepth, system, tkDisabled, tkAccent],
  ([$c, $t, $m, $s, $td, $ta]) => {
    const pages: OverridePage[] = [
      { key: "colours", labelKey: "s.override.colours", href: "/appearance/colors", count: Object.keys($c).length },
      { key: "typography", labelKey: "s.override.typography", href: "/appearance/typography", count: Object.keys($t).length },
      { key: "motion", labelKey: "s.override.motion", href: "/appearance/motion-depth", count: Object.keys($m).length },
      { key: "system", labelKey: "s.override.system", href: "/appearance/system", count: Object.keys($s).length },
      { key: "toolkits", labelKey: "s.override.toolkits", href: "/appearance/toolkits", count: Object.keys($td).length + Object.keys($ta).length },
    ];
    return { pages, total: pages.reduce((n, p) => n + p.count, 0) };
  },
);

/// Whether the summary could not be read, as opposed to being empty.
///
/// The two look identical and mean opposite things. Every loader below swallows
/// its own failure and leaves its store alone - correctly, so a page does not
/// blank its rows over an unreadable file - but a summary built from stores that
/// all quietly failed counts zero and renders "Nothing changed yet", which is a
/// statement about the machine made without asking it.
///
/// And the failure is not hypothetical. `theme.toml` is one file; a malformed
/// one makes every override read fail at once, and it is the same file whose
/// unparseability takes the whole theme down. So the person whose theme just
/// broke would open this page, be told they had customised nothing, and have
/// nothing to click.
export const summaryUnread = writable(false);

/// Fill every counted store from the machine, so the summary is about this
/// desktop rather than about which pages happen to have been opened.
///
/// Each loader is the page's own, not a second reader written here: a summary
/// with its own idea of where an override lives drifts from the page it links
/// to, and a drifted count is worse than no count because it is believed.
///
/// One read is made directly first, as the probe. The loaders cannot report
/// failure - they are built not to - so the only way to tell an empty answer
/// from an unanswered one is to ask something once and watch. The colour
/// overrides are the right question because they read the same `theme.toml` the
/// summary is about.
export async function loadSummary(): Promise<void> {
  // Under vite there is no host and every read throws, which is a dev condition
  // rather than a broken machine. The empty state is the one worth rendering
  // there; `loadReach` treats its own vite case the same way.
  if (!tauriAvailable) return;
  try {
    await invoke("theme_color_overrides");
    summaryUnread.set(false);
  } catch {
    summaryUnread.set(true);
    return;
  }
  await Promise.all([
    loadColorOverrides(),
    loadTypography(),
    loadMd(),
    loadSys(),
    loadEnabled(),
    loadToolkitAccents(),
  ]);
}

/// Clear every appearance override on the machine, then re-read what is left.
///
/// The write comes first and the stores follow it, so a refusal leaves the
/// summary showing what is still there. Re-reading rather than assuming empty is
/// the same discipline: the backend deliberately leaves `[accessibility]` alone,
/// so reduce-motion survives a reset and the Motion row honestly still counts it.
/// A store cleared to zero here would have claimed otherwise.
export async function resetAll(): Promise<void> {
  if (!tauriAvailable) return;
  await invoke("theme_reset_overrides");
  await loadSummary();
}
