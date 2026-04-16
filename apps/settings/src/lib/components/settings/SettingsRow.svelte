<script lang="ts">
  /// One row in a settings group: label on the left, control on the right,
  /// optional reset button when the value differs from the default.
  import type { Snippet } from "svelte";
  import { RotateCcw } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button";

  let {
    label,
    description,
    modified = false,
    onReset,
    control,
    below,
  }: {
    label: string;
    description?: string;
    modified?: boolean;
    onReset?: () => void;
    /// Right-side control (switch, slider, select, ...).
    control?: Snippet;
    /// Optional content rendered below the row (e.g. color swatch grid).
    below?: Snippet;
  } = $props();
</script>

<div class="px-4 py-3">
  <div class="flex items-center gap-4">
    <div class="flex-1 min-w-0">
      <div class="text-sm font-medium text-foreground">{label}</div>
      {#if description}
        <div class="text-xs text-muted-foreground">{description}</div>
      {/if}
    </div>
    <div class="flex items-center gap-2 shrink-0">
      {@render control?.()}
      {#if modified && onReset}
        <Button
          variant="ghost"
          size="icon-sm"
          onclick={onReset}
          aria-label="Reset to default"
          title="Reset to default"
        >
          <RotateCcw size={14} />
        </Button>
      {/if}
    </div>
  </div>
  {#if below}
    <div class="mt-3">
      {@render below()}
    </div>
  {/if}
</div>
