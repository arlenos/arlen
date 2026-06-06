import { Popover as PopoverPrimitive } from "bits-ui";

export { default as Content } from "./popover-content.svelte";

// Re-export bits-ui primitives. These are Svelte component classes,
// aliased through intermediate variables so Vite HMR can track them.
const Root = PopoverPrimitive.Root;
const Trigger = PopoverPrimitive.Trigger;

export { Root, Trigger };
