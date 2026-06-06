/// Component index for reference only.
///
/// shadcn-svelte components export overlapping names (Root, Content,
/// Trigger, Separator, etc.) across different component directories,
/// so a flat `export *` barrel creates ambiguity. Import per-directory
/// instead:
///
///   import { Button } from '$lib/components/ui/button';
///   import { FillSlider } from '$lib/components/ui/fill-slider';
///
/// Consumption (validated June 2026, see docs/architecture/design-system.md §2.1):
/// consuming apps import these directly via the `@arlen/ui-kit` SvelteKit
/// alias (`svelte.config.js`: "@arlen/ui-kit" -> "../sdk/ui-kit/src/lib"),
/// e.g. `import { Page } from "@arlen/ui-kit/components/ui/page"`. This is the
/// single source — do NOT copy these into apps.
///   - Scoped-style components (Page, SectionGrid, Group, Row, Switch,
///     SegmentedControl, ChipList, Toolbar, StatGrid, topbar, …) import cleanly
///     cross-crate with no extra setup (desktop-shell + app-settings both do).
///   - Tailwind-utility-class components (the shadcn-derived button/card/etc.)
///     additionally need the consuming app's `app.css` to scan this source via
///     `@source "../../sdk/ui-kit/src/lib/components/**/*.{svelte,ts}"`, so their
///     classes are generated; until each app adds that, those specific ones are
///     still copied (the S-U1b consolidation removes the copies).

// Re-export only the custom Arlen components that have unique names
// and NO app-specific store imports. Components that depend on
// `$lib/stores/theme` etc. stay in their respective apps.
export { ConfirmDialog } from "./confirm-dialog";
export { DaysPicker } from "./days-picker";
export { FillSlider } from "./fill-slider";
export { Group } from "./group";
export { NumberInput } from "./number-input";
export { Page } from "./page";
export { PopoverSelect, type PopoverSelectOption } from "./popover-select";
export { PositionPicker } from "./position-picker";
export { ChipList } from "./chip-list";
export { Row } from "./row";
export { SectionGrid } from "./section-grid";
export { SegmentedControl } from "./segmented-control";
export { StatGrid, StatTile } from "./stat-grid";
export { TimeInput } from "./time-input";
export { Toolbar } from "./toolbar";
