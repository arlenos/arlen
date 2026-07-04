<script lang="ts">
  /// A live preview strip: a small set of real UI primitives (a window with a
  /// titlebar, text, buttons in their accent states, a selectable list, status
  /// dots, an input) rendered from the theme's EFFECTIVE colours. The colours are
  /// scoped to this container's own CSS variables, so editing a role updates only
  /// the preview, not the whole Settings app. It rides the roundness scale so the
  /// same strip reflects geometry edits later. Precedent: tweakcn; deliberately
  /// not a fake desktop.
  let { colors }: { colors: Record<string, string> } = $props();

  // Map the semantic roles to the preview's local variables. Missing roles fall
  // back to a sensible sibling so a partial palette still renders.
  const vars = $derived(
    [
      ["--pv-bg", colors.bg_app],
      ["--pv-surface", colors.bg_card],
      ["--pv-overlay", colors.bg_overlay ?? colors.bg_card],
      ["--pv-input", colors.bg_input ?? colors.bg_card],
      ["--pv-accent", colors.accent],
      ["--pv-accent-hover", colors.accent_hover ?? colors.accent],
      ["--pv-accent-pressed", colors.accent_pressed ?? colors.accent],
      ["--pv-accent-fg", colors.fg_inverse ?? "#0a0a0a"],
      ["--pv-fg", colors.fg_primary],
      ["--pv-fg2", colors.fg_secondary ?? colors.fg_primary],
      ["--pv-border", colors.border_default],
      ["--pv-success", colors.success],
      ["--pv-warning", colors.warning],
      ["--pv-error", colors.error],
    ]
      .filter(([, v]) => v)
      .map(([k, v]) => `${k}:${v}`)
      .join(";"),
  );
</script>

<div class="pv" style={vars}>
  <div class="pv-window">
    <div class="pv-titlebar">
      <span class="pv-traffic"></span>
      <span class="pv-traffic"></span>
      <span class="pv-traffic"></span>
      <span class="pv-titletext">Preview</span>
    </div>
    <div class="pv-body">
      <div class="pv-copy">
        <span class="pv-h">The quick brown fox</span>
        <span class="pv-p">Secondary text sits a little quieter than the heading.</span>
      </div>

      <div class="pv-btns">
        <span class="pv-btn pv-primary">Button</span>
        <span class="pv-btn pv-primary pv-hover">Hover</span>
        <span class="pv-btn pv-primary pv-pressed">Pressed</span>
        <span class="pv-btn pv-secondary">Secondary</span>
      </div>

      <div class="pv-input">Text input</div>

      <div class="pv-list">
        <div class="pv-row pv-selected">Selected item</div>
        <div class="pv-row">List item</div>
      </div>

      <div class="pv-status">
        <span class="pv-dot" style="background: var(--pv-success)"></span>
        <span class="pv-dot" style="background: var(--pv-warning)"></span>
        <span class="pv-dot" style="background: var(--pv-error)"></span>
      </div>
    </div>
  </div>
</div>

<style>
  .pv {
    padding: 1rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .pv-window {
    border-radius: var(--radius-card, 12px);
    overflow: hidden;
    background: var(--pv-bg);
    border: 1px solid var(--pv-border);
  }
  .pv-titlebar {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.5rem 0.75rem;
    background: var(--pv-surface);
    border-bottom: 1px solid var(--pv-border);
  }
  .pv-traffic {
    width: 0.5rem;
    height: 0.5rem;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--pv-fg) 25%, transparent);
  }
  .pv-titletext {
    margin-left: 0.375rem;
    font-size: 0.6875rem;
    color: var(--pv-fg2);
  }
  .pv-body {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 0.875rem;
    background: var(--pv-bg);
  }
  .pv-copy {
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
  }
  .pv-h {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--pv-fg);
  }
  .pv-p {
    font-size: 0.75rem;
    color: var(--pv-fg2);
  }
  .pv-btns {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .pv-btn {
    font-size: 0.75rem;
    font-weight: 500;
    padding: 0.3125rem 0.75rem;
    border-radius: var(--radius-button, 6px);
    border: 1px solid transparent;
  }
  .pv-primary {
    background: var(--pv-accent);
    color: var(--pv-accent-fg);
  }
  .pv-primary.pv-hover {
    background: var(--pv-accent-hover);
  }
  .pv-primary.pv-pressed {
    background: var(--pv-accent-pressed);
  }
  .pv-secondary {
    background: var(--pv-surface);
    color: var(--pv-fg);
    border-color: var(--pv-border);
  }
  .pv-input {
    font-size: 0.75rem;
    color: var(--pv-fg2);
    padding: 0.375rem 0.625rem;
    border-radius: var(--radius-input, 8px);
    background: var(--pv-input);
    border: 1px solid var(--pv-border);
  }
  .pv-list {
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-input, 8px);
    overflow: hidden;
    border: 1px solid var(--pv-border);
  }
  .pv-row {
    font-size: 0.75rem;
    color: var(--pv-fg);
    padding: 0.375rem 0.625rem;
  }
  .pv-selected {
    background: color-mix(in srgb, var(--pv-accent) 22%, transparent);
    color: var(--pv-fg);
  }
  .pv-status {
    display: flex;
    gap: 0.375rem;
  }
  .pv-dot {
    width: 0.625rem;
    height: 0.625rem;
    border-radius: var(--radius-full, 9999px);
  }
</style>
