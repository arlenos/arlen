<script lang="ts">
  /// Centred dialog box.
  ///
  /// Wraps the bits-ui `Dialog.Content` primitive with our own
  /// styling and z-layering. The content sits at `z-[500]`, one
  /// above the `Overlay` (490), and inherits the shell-popover
  /// scope so all shadcn child components pick up the dialog-
  /// appropriate token palette (solid card background, shell-
  /// foreground text colour) instead of the default app-window
  /// palette.
  ///
  /// `escapeKeydownBehavior` and `interactOutsideBehavior` default
  /// to "close" — Escape and backdrop click both dismiss. Consumers
  /// that need to intercept either (e.g. confirm-before-close on a
  /// destructive form) override via the bits-ui API; bluetooth
  /// pairing uses the defaults so users always have an out.

  import { cn } from "$lib/utils.js";
  import { Dialog as DialogPrimitive } from "bits-ui";
  import DialogOverlay from "./dialog-overlay.svelte";

  let {
    ref = $bindable(null),
    class: className,
    children,
    portalProps,
    ...restProps
  }: DialogPrimitive.ContentProps & {
    portalProps?: DialogPrimitive.PortalProps;
  } = $props();
</script>

<DialogPrimitive.Portal {...portalProps}>
  <DialogOverlay />
  <DialogPrimitive.Content
    bind:ref
    data-slot="dialog-content"
    class={cn(
      "shell-popover fixed left-1/2 top-1/2 z-[500] w-full max-w-md -translate-x-1/2 -translate-y-1/2 rounded-[var(--radius)] border border-border bg-[var(--color-bg-card)] p-5 shadow-2xl outline-none data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0 data-[state=open]:zoom-in-95 data-[state=closed]:zoom-out-95",
      className,
    )}
    {...restProps}
  >
    {@render children?.()}
  </DialogPrimitive.Content>
</DialogPrimitive.Portal>
