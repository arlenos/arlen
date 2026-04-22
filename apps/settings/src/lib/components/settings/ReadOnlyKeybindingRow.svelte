<script lang="ts">
  /// Read-only counterpart to `KeybindingRow` for bindings that are
  /// NOT managed by the compositor's keybinding system — e.g. the
  /// internal Workspace Map navigation keys that the desktop-shell
  /// frontend handles directly. The visual is intentionally
  /// indistinguishable from the editable rows (same padding, same
  /// mono pill on the right, same label layout) so users don't see
  /// a style seam when a category mixes both kinds. The one
  /// difference: the pill isn't a button — no rebind, no reset,
  /// no trash. Hover styles are also omitted to reinforce "this
  /// isn't interactive".

  type Props = {
    label: string;
    keys: string;
    description?: string;
  };

  let { label, keys, description }: Props = $props();
</script>

<div class="flex items-center gap-3 rounded-[var(--radius-sm)] border border-transparent px-3 py-2">
  <div class="min-w-0 flex-1">
    <div class="flex items-center gap-2">
      <span class="text-sm font-medium">{label}</span>
    </div>
    {#if description}
      <div class="mt-0.5 text-xs text-muted-foreground">
        {description}
      </div>
    {/if}
  </div>

  <div
    class="inline-flex min-w-20 items-center justify-center rounded-[var(--radius-sm)] border border-border bg-background px-3 py-1 font-mono text-xs text-muted-foreground"
  >
    {keys}
  </div>
</div>
