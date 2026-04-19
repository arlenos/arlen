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
/// Consuming apps (desktop-shell, app-settings) keep file copies of
/// each component under their own `src/lib/components/ui/` directory.
/// Tailwind's scope-hashing breaks if the file is imported across
/// crate boundaries, so we sync via copy rather than symlink. When
/// you change a canonical component here, mirror the change into
/// both `app-settings/src/lib/components/ui/<name>/` and
/// `desktop-shell/src/lib/components/ui/<name>/`.

// Re-export only the custom Lunaris components that have unique names
// and NO app-specific store imports. Components that depend on
// `$lib/stores/theme` etc. stay in their respective apps.
export { ConfirmDialog } from "./confirm-dialog";
export { DaysPicker } from "./days-picker";
export { Group } from "./group";
export { NumberInput } from "./number-input";
export { PopoverSelect, type PopoverSelectOption } from "./popover-select";
export { PositionPicker } from "./position-picker";
export { Row } from "./row";
export { TimeInput } from "./time-input";
export { ValueSlider } from "./value-slider";
