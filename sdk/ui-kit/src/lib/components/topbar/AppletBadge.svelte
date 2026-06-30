<script lang="ts">
  /// Standard corner badge for an Applet primitive.
  ///
  /// Top-right overlay chip with three variants:
  ///   * `count` — numeric pill (notification count, etc.).
  ///     Auto-formats `> 99` to `99+`. Min-width 14, grows for
  ///     wide numbers, height 14.
  ///   * `dot` — small coloured dot (presence indicator).
  ///   * `icon` — small icon (charging-bolt for battery, lock for
  ///     locked WiFi, etc.).
  ///
  /// Colour is one of the semantic shell tokens via the `color`
  /// prop. Default = `accent`.
  ///
  /// Sits in `.applet-badge-slot` (own stacking context — see
  /// Applet.svelte) so the badge stays crisp even when the parent
  /// applet is `dimmed`.
  import type { Snippet } from "svelte";

  type Variant = "count" | "dot" | "icon";
  type BadgeColor = "accent" | "success" | "warn" | "error";

  let {
    variant,
    value,
    color = "accent",
    icon,
  }: {
    variant: Variant;
    /// For `variant="count"`. Numbers `> 99` render as `99+`.
    value?: number;
    color?: BadgeColor;
    /// For `variant="icon"`. Caller passes a sized lucide icon.
    icon?: Snippet;
  } = $props();

  const text = $derived(
    variant === "count" && typeof value === "number"
      ? value > 99
        ? "99+"
        : String(value)
      : "",
  );
</script>

{#if variant === "count" && typeof value === "number" && value > 0}
  <span class="badge count" class:c-accent={color === "accent"} class:c-success={color === "success"} class:c-warn={color === "warn"} class:c-error={color === "error"}>{text}</span>
{:else if variant === "dot"}
  <span class="badge dot" class:c-accent={color === "accent"} class:c-success={color === "success"} class:c-warn={color === "warn"} class:c-error={color === "error"}></span>
{:else if variant === "icon" && icon}
  <span class="badge icon" class:c-accent={color === "accent"} class:c-success={color === "success"} class:c-warn={color === "warn"} class:c-error={color === "error"}>{@render icon()}</span>
{/if}

<style>
  .badge {
    position: absolute;
    top: 1px;
    right: 1px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    box-sizing: border-box;
    border-radius: var(--radius-full, 9999px);
    /* The badge sits in the parent's `.applet-badge-slot` which is
       absolute-positioned. Pulling it slightly outside the icon's
       bounds (top: 1, right: 1) keeps it from clipping when the
       icon's box-shadow / outline grows on focus. */
  }

  .count {
    min-width: 14px;
    height: 14px;
    padding: 0 4px;
    font-size: 0.5625rem;
    font-weight: 700;
    line-height: 14px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .dot {
    width: 8px;
    height: 8px;
    /* The dot variant rides the roundness scale; the count/icon badges
       keep the pill radius from `.badge`. */
    border-radius: var(--radius-chip, 4px);
  }
  .icon {
    width: 14px;
    height: 14px;
  }

  /* Colour variants — same source-of-truth as the rest of the
     shell. The text colour for filled badges uses the contrasting
     foreground if the theme provides one, else white. */
  .c-accent {
    background: var(--color-accent);
    color: var(--color-accent-foreground, #ffffff);
  }
  .c-success {
    background: var(--color-success, #10b981);
    color: var(--color-success-foreground, #ffffff);
  }
  .c-warn {
    background: var(--color-warning, #eab308);
    color: var(--color-warning-foreground, #ffffff);
  }
  .c-error {
    background: var(--color-error, #ef4444);
    color: var(--color-error-foreground, #ffffff);
  }
</style>
