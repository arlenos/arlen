/// Reference index for shell-LOCAL ui components only.
///
/// The shared design-system canon (Button, Switch, Group, Row, popover,
/// select, sidebar, inputs, …) lives in `sdk/ui-kit` and is imported directly
/// via the `@arlen/ui-kit` alias, e.g.
///   import { Button } from "@arlen/ui-kit/components/ui/button";
/// Do NOT copy those here. Only shell-specific components with no ui-kit canon
/// stay under this directory (currently: dialog).
///
/// Import per-directory (shadcn names overlap, so no flat `export *`).
