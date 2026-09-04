<script lang="ts">
  /// A single-line label that reveals its full text on hover ONLY when it is
  /// actually truncated (design-system.md §6.4): the reveal goes through the
  /// kit tooltip, carries the same string and nothing more, and never through
  /// `title=`. A label the reader can already see in full gets no tooltip.
  ///
  /// The element measures itself (`scrollWidth` against `clientWidth`, as the
  /// breadcrumb does) and re-measures on resize, so a widened window takes the
  /// tooltip away again. Place it inside the box that sets the width; it
  /// fills that box as a block and clips with an ellipsis.
  import * as Tooltip from "../tooltip/index.js";

  let {
    text,
    id,
    class: className,
  }: {
    text: string;
    id?: string;
    class?: string;
  } = $props();

  let el = $state<HTMLElement | null>(null);
  let over = $state(false);

  function measure(): void {
    if (el) over = el.scrollWidth > el.clientWidth + 1;
  }

  $effect(() => {
    void text;
    if (!el) return;
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });
</script>

{#if over}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <span bind:this={el} class="overflow-label {className ?? ''}" {id} {...props}>{text}</span>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.TooltipContent>{text}</Tooltip.TooltipContent>
  </Tooltip.Root>
{:else}
  <span bind:this={el} class="overflow-label {className ?? ''}" {id}>{text}</span>
{/if}

<style>
  .overflow-label {
    display: block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
