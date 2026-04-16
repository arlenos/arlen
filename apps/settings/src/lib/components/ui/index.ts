/// Component index for reference only.
///
/// shadcn-svelte components export overlapping names (Root, Content,
/// Trigger, Separator, etc.) across different component directories,
/// so a flat `export *` barrel creates ambiguity. Import per-directory
/// instead:
///
///   import { Button } from '$lib/components/ui/button';
///   import { ValueSlider } from '$lib/components/ui/value-slider';
///
/// Consuming apps (desktop-shell, app-settings) symlink their own
/// `src/lib/components/ui/` to this directory so the standard
/// `$lib/components/ui/X` import path resolves here.

// Re-export only the custom Lunaris components that have unique names
// and NO app-specific store imports. Components that depend on
// `$lib/stores/theme` etc. stay in their respective apps.
export { DaysPicker } from "./days-picker";
export { Group } from "./group";
export { PositionPicker } from "./position-picker";
export { Row } from "./row";
export { TimeInput } from "./time-input";
export { ValueSlider } from "./value-slider";
