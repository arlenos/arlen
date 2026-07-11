<script lang="ts">
  /// The operation surfaces: the conflict dialog (skip / keep both /
  /// replace), the progress veil for operations that take a moment,
  /// and the quiet error line. All driven by the ops store; no
  /// component talks to the backend directly. The conflict dialog rides
  /// the shared modal shell so it reads as one surface with the rest.
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { conflict, opBusy, opError } from "$lib/stores/ops";
  import { t } from "$lib/i18n/messages";
</script>

{#if $opBusy}
  <div class="op-veil" role="status" aria-label={$opBusy}>
    <div class="op-card">
      <span class="op-spinner" aria-hidden="true"></span>
      <span class="op-label">{$opBusy}</span>
    </div>
  </div>
{/if}

{#if $conflict}
  <Dialog open onClose={() => conflict.set(null)} ariaLabel={$t("f.ops.conflictAria")} size="md">
    <div class="op-conflict-body">
      <span class="op-title">{$t("f.ops.conflict", { name: $conflict.name })}</span>
      <span class="op-hint">{$t("f.ops.conflictPrompt")}</span>
      <div class="op-actions">
        <Button variant="ghost" onclick={() => conflict.set(null)}>{$t("f.ops.cancel")}</Button>
        <span class="op-actions-spacer"></span>
        <Button variant="outline" onclick={() => $conflict?.retry("skip")}>{$t("f.ops.skip")}</Button>
        <Button variant="outline" onclick={() => $conflict?.retry("rename")}>
          {$t("f.ops.keepBoth")}
        </Button>
        <Button variant="destructive" onclick={() => $conflict?.retry("replace")}>
          {$t("f.ops.replace")}
        </Button>
      </div>
    </div>
  </Dialog>
{/if}

{#if $opError}
  <div class="op-errorline" role="alert">
    <span>{$opError}</span>
    <button class="op-dismiss" onclick={() => opError.set(null)}>{$t("f.dismiss")}</button>
  </div>
{/if}

<style>
  /* The progress veil is a status overlay (a spinner + label), not a dialog, so
     it keeps its own light frame rather than the modal shell. */
  .op-veil {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-bg-overlay);
  }
  .op-card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 20px;
    border: 1px solid color-mix(in srgb, var(--foreground) 15%, transparent);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg);
  }
  .op-spinner {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-full);
    border: 2px solid color-mix(in srgb, var(--foreground) 20%, transparent);
    border-top-color: var(--color-accent, var(--primary));
    animation: op-spin 0.8s linear infinite;
  }
  @keyframes op-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .op-label {
    font-size: var(--text-xs);
    color: var(--foreground);
  }

  /* The conflict dialog's body, inside the shared modal shell. */
  .op-conflict-body {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 16px 20px;
  }
  .op-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    overflow-wrap: anywhere;
  }
  .op-hint {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .op-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    margin-top: 12px;
  }
  .op-actions-spacer {
    flex: 1;
  }

  .op-errorline {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 16px;
    border-top: 1px solid color-mix(in srgb, var(--color-error) 30%, transparent);
    background: color-mix(in srgb, var(--color-error) 8%, transparent);
    font-size: var(--text-xs);
    color: var(--foreground);
  }
  .op-errorline span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .op-dismiss {
    height: var(--height-control-compact, 24px);
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-chip);
    background: transparent;
    color: var(--foreground);
    font-size: var(--text-xs);
  }
  .op-dismiss:hover {
    background: var(--control-bg-hover);
  }
</style>
