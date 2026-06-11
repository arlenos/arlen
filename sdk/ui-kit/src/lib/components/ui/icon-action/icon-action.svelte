<script lang="ts">
  /// Icon-only action button with a themed tooltip: the quiet ghost
  /// register for secondary actions (a message's hover row, a list row's
  /// chevron, a group label's add button). The tooltip is the kit one, not
  /// the browser-native title bubble; the label doubles as the accessible
  /// name.
  import type { Snippet } from "svelte";
  import * as Tooltip from "../tooltip/index.js";

  let {
    label,
    id,
    size = "compact",
    active = false,
    disabled = false,
    onclick,
    children,
  }: {
    /// Tooltip text and accessible name.
    label: string;
    /// Optional anchor id (deep-link/search canon).
    id?: string;
    /// 24px (hover/secondary) or 28px (inline control register).
    size?: "compact" | "control";
    /// Persistent on-state (a set bookmark).
    active?: boolean;
    disabled?: boolean;
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
  } = $props();
</script>

<Tooltip.Root instant>
  <Tooltip.Trigger>
    {#snippet child({ props })}
      {@const { onclick: triggerClick, ...rest } = props as Record<string, unknown>}
      <!-- The trigger's own click handler is chained, not clobbered, and the
           explicit id wins over the generated one so canon anchor ids stay
           stable; the tooltip still tracks the element. -->
      <button
        type="button"
        class="ia"
        data-size={size}
        class:active
        aria-label={label}
        {disabled}
        {...rest}
        {...id ? { id } : {}}
        onclick={(e: MouseEvent) => {
          (triggerClick as ((ev: MouseEvent) => void) | undefined)?.(e);
          onclick?.(e);
        }}
      >
        {@render children?.()}
      </button>
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.TooltipContent>{label}</Tooltip.TooltipContent>
</Tooltip.Root>

<style>
  .ia {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    border-radius: var(--radius-button);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .ia[data-size="compact"] {
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
  }
  .ia[data-size="control"] {
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
  }
  .ia:hover:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .ia.active {
    color: var(--foreground);
  }
  .ia:disabled {
    opacity: 0.4;
  }
</style>
