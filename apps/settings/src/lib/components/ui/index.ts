/// Reference index for the app-LOCAL ui components only.
///
/// The shared design-system canon (Page, SectionGrid, Group, Row, Button,
/// Switch, inputs, AddRemoveList, ValueSlider, DirectoryPicker, …) lives in
/// `sdk/ui-kit` and is imported directly via the `@arlen/ui-kit` alias, e.g.
///   import { Button } from "@arlen/ui-kit/components/ui/button";
/// Do NOT copy those here. Only app-specific components that have no ui-kit
/// canon stay under this directory: command-string-editor (compositor
/// command schema) and skeleton (QuickSettings layout mocks).
///
/// Import per-directory (shadcn names overlap, so no flat `export *`).
