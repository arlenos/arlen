<script lang="ts">
  /// A live preview: real @arlen/ui-kit primitives on a themed surface, scoped to
  /// the theme's EFFECTIVE tokens. Editing a role sets this container's own
  /// --color-* variables, which cascade through the app's shadcn aliases
  /// (--primary: var(--color-accent), ...) into the real components, so only the
  /// preview re-themes, not the whole Settings app. It inherits the page's
  /// --radius-* / --font-* too, so it also reflects geometry + typography edits.
  /// Precedent: tweakcn; real components, deliberately not a fake desktop.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";

  let { colors }: { colors: Record<string, string> } = $props();

  // Scope the effective palette onto this container. The components read the
  // shadcn tokens directly (Tailwind `@theme inline` inlines `bg-primary` to
  // `var(--primary)`), and those tokens are resolved to literals at :root and
  // inherited - so overriding the source `--color-*` here would NOT re-resolve
  // them. We set the shadcn tokens themselves on the container; the badges' status
  // colours read `--color-success/warning/error` directly, so those stay too.
  const inverse = $derived(colors.fg_inverse ?? "#0a0a0a");
  const surface = $derived(colors.bg_card);
  const vars = $derived(
    [
      ["--background", colors.bg_app],
      ["--foreground", colors.fg_primary],
      ["--card", surface],
      ["--card-foreground", colors.fg_primary],
      ["--popover", surface],
      ["--popover-foreground", colors.fg_primary],
      ["--primary", colors.accent],
      ["--primary-foreground", inverse],
      ["--secondary", surface],
      ["--secondary-foreground", colors.fg_primary],
      ["--muted", surface],
      ["--muted-foreground", colors.fg_secondary ?? colors.fg_primary],
      ["--accent", colors.accent],
      ["--accent-foreground", inverse],
      ["--destructive", colors.error],
      ["--border", colors.border_default],
      ["--input", colors.bg_input ?? surface],
      ["--ring", colors.accent],
      ["--color-success", colors.success],
      ["--color-warning", colors.warning],
      ["--color-error", colors.error],
    ]
      .filter(([, v]) => v)
      .map(([k, v]) => `${k}:${v}`)
      .join(";"),
  );
</script>

<div class="pv" style={vars}>
  <div class="pv-surface">
    <div class="pv-copy">
      <span class="pv-h">The quick brown fox</span>
      <span class="pv-p">Secondary text sits a little quieter than the heading.</span>
    </div>

    <div class="pv-row">
      <Button>Primary</Button>
      <Button variant="secondary">Secondary</Button>
      <Button variant="outline">Outline</Button>
    </div>

    <Input placeholder="Text input" readonly tabindex={-1} />

    <div class="pv-list">
      <div class="pv-item pv-selected">Selected item</div>
      <div class="pv-item">List item</div>
    </div>

    <div class="pv-badges">
      <Badge>Default</Badge>
      <Badge variant="success">Success</Badge>
      <Badge variant="warn">Warning</Badge>
    </div>
  </div>
</div>

<style>
  .pv {
    color: var(--foreground);
  }
  /* The themed surface: a real card built from the resolved tokens (which, inside
     .pv, resolve to the edited palette). Rides the card radius + elevation. */
  .pv-surface {
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
    padding: 1rem;
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    box-shadow: var(--shadow-card, none);
  }
  .pv-copy {
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
  }
  .pv-h {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .pv-p {
    font-size: 0.75rem;
    color: var(--muted-foreground);
  }
  .pv-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .pv-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-input, 8px);
    overflow: hidden;
  }
  .pv-item {
    padding: 0.375rem 0.625rem;
    font-size: 0.75rem;
    color: var(--foreground);
  }
  .pv-selected {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .pv-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
  }
</style>
