/// The cross-page "my customisations" summary for the Appearance landing: how
/// many fields the user has overridden on top of the active theme, broken down by
/// page, and a reset-all. Reads the six suite stores' override state.

import { derived } from "svelte/store";
import { overrides as colours } from "./themeColors";
import { overrides as geometry, smartGaps, smartGapsOverridden } from "./themeGeometry";
import { overrides as typography } from "./themeTypography";
import { overrides as motionDepth } from "./themeMotionDepth";
import { overrides as system } from "./themeSystem";
import { disabled as tkDisabled, accentOverrides as tkAccent } from "./themeToolkits";

/// One page's override count + where to reach it.
export interface OverridePage {
  key: string;
  label: string;
  href: string;
  count: number;
}

/// The live summary: the per-page counts and the total.
export const overrideSummary = derived(
  [colours, geometry, smartGapsOverridden, typography, motionDepth, system, tkDisabled, tkAccent],
  ([$c, $g, $sg, $t, $m, $s, $td, $ta]) => {
    const pages: OverridePage[] = [
      { key: "colours", label: "Colours", href: "/appearance/colors", count: Object.keys($c).length },
      { key: "geometry", label: "Geometry", href: "/appearance/geometry", count: Object.keys($g).length + ($sg ? 1 : 0) },
      { key: "typography", label: "Typography", href: "/appearance/typography", count: Object.keys($t).length },
      { key: "motion", label: "Motion & Depth", href: "/appearance/motion-depth", count: Object.keys($m).length },
      { key: "system", label: "System", href: "/appearance/system", count: Object.keys($s).length },
      { key: "toolkits", label: "Toolkits", href: "/appearance/toolkits", count: Object.keys($td).length + Object.keys($ta).length },
    ];
    return { pages, total: pages.reduce((n, p) => n + p.count, 0) };
  },
);

/// Clear every override across the suite, back to the theme's values.
export function resetAll(): void {
  colours.set({});
  geometry.set({});
  smartGaps.set(true);
  smartGapsOverridden.set(false);
  typography.set({});
  motionDepth.set({});
  system.set({});
  tkDisabled.set({});
  tkAccent.set({});
}
