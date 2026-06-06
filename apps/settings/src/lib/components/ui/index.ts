/// Reference index for the app-LOCAL ui components only.
///
/// The shared design-system canon (Page, SectionGrid, Group, Row, Button,
/// Switch, inputs, …) now lives in `sdk/ui-kit` and is imported directly via
/// the `@arlen/ui-kit` alias, e.g.
///   import { Button } from "@arlen/ui-kit/components/ui/button";
/// Do NOT copy those here. Only app-specific components that have no ui-kit
/// canon stay under this directory (add-remove-list, command-string-editor,
/// directory-picker, skeleton, value-slider) plus the complex multi-file
/// shadcn components still pending the cross-crate consolidation (command,
/// context-menu, dropdown-menu, popover, popover-select, select, sidebar).
///
/// Import per-directory (shadcn names overlap, so no flat `export *`).

export { ValueSlider } from "./value-slider";
