/// Generic Shadcn-style Dialog primitive for desktop-shell.
///
/// Wraps bits-ui's Dialog primitives with our token-based styling
/// and z-layering (overlay 490, content 500). The dialog scope
/// is `.shell-popover` so all child shadcn components inherit the
/// shell-popover token palette automatically.
///
/// Used by `BluetoothPairingDialog`. Other call sites in the
/// desktop-shell (currently the only one) follow the same pattern.

import { Dialog as DialogPrimitive } from "bits-ui";

export { default as Content } from "./dialog-content.svelte";
export { default as Description } from "./dialog-description.svelte";
export { default as Footer } from "./dialog-footer.svelte";
export { default as Header } from "./dialog-header.svelte";
export { default as Overlay } from "./dialog-overlay.svelte";
export { default as Title } from "./dialog-title.svelte";

const Root = DialogPrimitive.Root;
const Trigger = DialogPrimitive.Trigger;
const Portal = DialogPrimitive.Portal;
const Close = DialogPrimitive.Close;

export { Close, Portal, Root, Trigger };
