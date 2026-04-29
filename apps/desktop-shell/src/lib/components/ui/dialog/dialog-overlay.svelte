<script lang="ts">
  /// Backdrop element rendered behind the dialog content.
  ///
  /// Sits on `z-[490]`, exactly one layer below the dialog content,
  /// so context menus (300) and popovers (50-100) are visually
  /// covered. Click-through is intentional: clicking on the overlay
  /// dismisses the dialog through bits-ui's outside-click handler.
  ///
  /// Background is the existing `--color-bg-overlay` token (the
  /// same one popovers use behind themselves) so the backdrop sits
  /// in the established theme palette.

  import { cn } from "$lib/utils.js";
  import { Dialog as DialogPrimitive } from "bits-ui";

  let {
    ref = $bindable(null),
    class: className,
    ...restProps
  }: DialogPrimitive.OverlayProps = $props();
</script>

<DialogPrimitive.Overlay
  bind:ref
  data-slot="dialog-overlay"
  class={cn(
    "fixed inset-0 z-[490] bg-[var(--color-bg-overlay)] data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0",
    className,
  )}
  {...restProps}
/>
