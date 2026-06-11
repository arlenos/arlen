<script lang="ts">
  /// The operation surfaces: the conflict dialog (skip / keep both /
  /// replace), the progress veil for operations that take a moment,
  /// and the quiet error line. All driven by the ops store; no
  /// component talks to the backend directly.
  import { conflict, opBusy, opError } from "$lib/stores/ops";
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
  <div class="op-veil" role="presentation">
    <div
      class="op-card op-conflict"
      role="dialog"
      aria-modal="true"
      aria-label="Name conflict"
      tabindex="-1"
    >
      <span class="op-title">{$conflict.name} already exists here</span>
      <span class="op-hint">What should happen with it?</span>
      <div class="op-actions">
        <button class="op-btn" onclick={() => conflict.set(null)}>Cancel</button>
        <span class="op-actions-spacer"></span>
        <button class="op-btn" onclick={() => $conflict?.retry("skip")}>Skip</button>
        <button class="op-btn" onclick={() => $conflict?.retry("rename")}>
          Keep both
        </button>
        <button class="op-btn destructive" onclick={() => $conflict?.retry("replace")}>
          Replace
        </button>
      </div>
    </div>
  </div>
{/if}

{#if $opError}
  <div class="op-errorline" role="alert">
    <span>{$opError}</span>
    <button class="op-dismiss" onclick={() => opError.set(null)}>Dismiss</button>
  </div>
{/if}

<style>
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
  .op-conflict {
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    width: min(380px, calc(100vw - 48px));
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
    to { transform: rotate(360deg); }
  }
  .op-label {
    font-size: 0.75rem;
    color: var(--foreground);
  }

  .op-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    overflow-wrap: anywhere;
  }
  .op-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .op-actions {
    display: flex;
    gap: 8px;
    width: 100%;
    margin-top: 12px;
  }
  .op-actions-spacer {
    flex: 1;
  }
  .op-btn {
    height: var(--height-control, 28px);
    padding: 0 12px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .op-btn:hover {
    background: var(--control-bg-hover);
  }
  .op-btn.destructive {
    border-color: color-mix(in srgb, var(--color-error) 40%, transparent);
    color: var(--color-error);
  }
  .op-btn.destructive:hover {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
  }

  .op-errorline {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 16px;
    border-top: 1px solid color-mix(in srgb, var(--color-error) 30%, transparent);
    background: color-mix(in srgb, var(--color-error) 8%, transparent);
    font-size: 0.75rem;
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
    font-size: 0.75rem;
  }
  .op-dismiss:hover {
    background: var(--control-bg-hover);
  }
</style>
