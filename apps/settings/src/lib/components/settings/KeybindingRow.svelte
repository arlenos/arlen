<script lang="ts">
  import { RotateCcw, Trash2, AlertTriangle } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button";
  import type { KeybindingEntry } from "$lib/stores/keybindings";

  type Props = {
    entry: KeybindingEntry;
    hasConflict: boolean;
    /// Fired when the user clicks the binding pill — UI opens KeyCapture.
    onRebind: (entry: KeybindingEntry) => void;
    /// Fired when the reset button is clicked.
    onReset: (entry: KeybindingEntry) => void;
    /// Fired when the remove (trash) button is clicked. Only shown for
    /// catalogue actions without a default (removable) and custom rows.
    onRemove: (entry: KeybindingEntry) => void;
  };

  let { entry, hasConflict, onRebind, onReset, onRemove }: Props = $props();

  const hasDefault = $derived(entry.defaultBinding !== null);
  const isModified = $derived(
    entry.binding !== (entry.defaultBinding ?? null)
  );
  const canRemove = $derived(!hasDefault || entry.category === "custom");
  /// Module bindings live in `compositor.d/keybindings.d/*.toml`. When
  /// the user rebinds one, the new accelerator is written to the main
  /// compositor.toml `[keybindings]` at User scope, which beats the
  /// Module-scope fragment. The row surfaces that as a hint so users
  /// don't wonder why "their" shortcut isn't the module's default.
  const showsModuleOverride = $derived(
    entry.category === "module" && isModified
  );
</script>

<div
  class="flex items-center gap-3 rounded-[var(--radius-chip)] border border-transparent px-3 py-2 transition-colors hover:border-border hover:bg-muted/30"
  class:bg-destructive={hasConflict}
  class:text-destructive-foreground={hasConflict}
  class:border-destructive={hasConflict}
>
  <div class="min-w-0 flex-1">
    <div class="flex items-center gap-2">
      <span class="text-sm font-medium">{entry.label}</span>
      {#if hasConflict}
        <AlertTriangle class="h-3.5 w-3.5 text-destructive" />
      {/if}
    </div>
    {#if entry.description}
      <div class="mt-0.5 text-xs text-muted-foreground">
        {entry.description}
      </div>
    {/if}
    {#if showsModuleOverride}
      <div class="mt-0.5 text-xs text-muted-foreground">
        Overrides module default
        {#if entry.defaultBinding}
          <span class="font-mono">{entry.defaultBinding}</span>
        {/if}
      </div>
    {/if}
  </div>

  <button
    type="button"
    class="inline-flex min-w-20 items-center justify-center rounded-[var(--radius-chip)] border border-border bg-background px-3 py-1 font-mono text-xs transition-colors hover:bg-muted"
    onclick={() => onRebind(entry)}
    aria-label="Change binding for {entry.label}"
  >
    {entry.binding ?? "Not set"}
  </button>

  {#if isModified && hasDefault}
    <Button
      variant="ghost"
      size="icon"
      onclick={() => onReset(entry)}
      aria-label="Reset to default"
      title="Reset to default ({entry.defaultBinding})"
    >
      <RotateCcw class="h-3.5 w-3.5" />
    </Button>
  {/if}

  {#if canRemove}
    <Button
      variant="ghost"
      size="icon"
      onclick={() => onRemove(entry)}
      aria-label="Remove binding"
    >
      <Trash2 class="h-3.5 w-3.5" />
    </Button>
  {/if}
</div>
