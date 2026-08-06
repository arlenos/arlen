<script lang="ts">
  /// Multi-line text input on the input register: same border, background,
  /// radius, and focus ring as `Input`, growing with content between the
  /// `rows` minimum and `maxRows`. For chat composers and note fields.
  import type { HTMLTextareaAttributes } from "svelte/elements";
  import { cn } from "../../../utils.js";

  let {
    ref = $bindable(null),
    value = $bindable(""),
    class: className,
    rows = 1,
    maxRows = 8,
    ...restProps
  }: HTMLTextareaAttributes & {
    ref?: HTMLTextAreaElement | null;
    value?: string;
    /// Minimum visible rows.
    rows?: number;
    /// Growth cap; beyond it the textarea scrolls.
    maxRows?: number;
  } = $props();

  /// Grow to fit the content up to `maxRows`, then scroll. Reads the
  /// computed line height so the cap follows the font. Skips while the
  /// element is hidden (a collapsed expander, a background tab): scrollHeight
  /// is 0 there, and fitting to it would nail the box to its padding.
  function autogrow(el: HTMLTextAreaElement) {
    if (el.clientWidth === 0) return;
    el.style.height = "auto";
    const line = parseFloat(getComputedStyle(el).lineHeight) || 20;
    const padding = el.offsetHeight - el.clientHeight;
    const max = line * maxRows + padding;
    el.style.height = `${Math.min(el.scrollHeight, max)}px`;
    el.style.overflowY = el.scrollHeight > max ? "auto" : "hidden";
  }

  // Re-fit on mount and on every value change, typed or programmatic (a
  // draft restored, a starter inserted). Watching the bound value instead of
  // an oninput attribute keeps the consumer's own oninput free.
  $effect(() => {
    void value;
    if (ref) autogrow(ref);
  });

  // Re-fit when the element's size changes from outside: most importantly the
  // 0 -> real-width jump when a hidden ancestor (Collapsible, tab) opens,
  // which the value-watching effect never sees. Settles immediately because
  // re-applying an unchanged height fires no further resize.
  $effect(() => {
    if (!ref) return;
    const el = ref;
    const ro = new ResizeObserver(() => autogrow(el));
    ro.observe(el);
    return () => ro.disconnect();
  });
</script>

<textarea
  bind:this={ref}
  bind:value
  {rows}
  data-slot="textarea"
  class={cn(
    "w-full resize-none rounded-input border border-border bg-input px-3 py-1.5 text-sm leading-5 transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50",
    className
  )}
  {...restProps}
></textarea>
