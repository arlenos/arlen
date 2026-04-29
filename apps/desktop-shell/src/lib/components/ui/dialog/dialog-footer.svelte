<script lang="ts">
  /// Right-aligned button row at the bottom of a dialog.
  ///
  /// `gap-2` matches the spacing that `ConfirmDialog` in ui-kit uses
  /// so visual rhythm stays consistent across the codebase.

  import { cn } from "$lib/utils.js";
  import type { Snippet } from "svelte";

  let {
    class: className,
    children,
  }: { class?: string; children?: Snippet } = $props();
</script>

<div
  data-slot="dialog-footer"
  class={cn("mt-4 flex justify-end gap-2", className)}
>
  {@render children?.()}
</div>
