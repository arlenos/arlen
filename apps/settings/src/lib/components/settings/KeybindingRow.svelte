<script lang="ts">
  import { RotateCcw, Trash2, AlertTriangle } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
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

<div class="kb-row" class:conflict={hasConflict}>
  <div class="kb-label">
    <div class="kb-title-line">
      <span class="kb-title">{entry.label}</span>
      {#if hasConflict}
        <AlertTriangle size={14} strokeWidth={2} class="kb-conflict-icon" />
      {/if}
    </div>
    {#if entry.description}
      <div class="kb-desc">{entry.description}</div>
    {/if}
    {#if showsModuleOverride}
      <div class="kb-desc">
        Overrides module default
        {#if entry.defaultBinding}
          <span class="kb-mono">{entry.defaultBinding}</span>
        {/if}
      </div>
    {/if}
  </div>

  <Button
    variant="outline"
    size="sm"
    class="kb-pill"
    onclick={() => onRebind(entry)}
    aria-label="Change binding for {entry.label}"
  >
    {entry.binding ?? "Not set"}
  </Button>

  {#if isModified && hasDefault}
    <Button
      variant="ghost"
      size="icon"
      onclick={() => onReset(entry)}
      aria-label="Reset to default"
      title="Reset to default ({entry.defaultBinding})"
    >
      <RotateCcw size={14} strokeWidth={2} />
    </Button>
  {/if}

  {#if canRemove}
    <Button variant="ghost" size="icon" onclick={() => onRemove(entry)} aria-label="Remove binding">
      <Trash2 size={14} strokeWidth={2} />
    </Button>
  {/if}
</div>

<style>
  .kb-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-chip);
    border: 1px solid transparent;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }
  .kb-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .kb-row.conflict {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    border-color: color-mix(in srgb, var(--color-error) 35%, transparent);
  }
  .kb-label {
    min-width: 0;
    flex: 1;
  }
  .kb-title-line {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .kb-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  :global(.kb-conflict-icon) {
    color: var(--color-error);
    flex-shrink: 0;
  }
  .kb-desc {
    margin-top: 0.0625rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .kb-mono,
  :global(.kb-pill) {
    font-family: var(--font-mono, monospace);
  }
  :global(.kb-pill) {
    min-width: 5rem;
  }
</style>
