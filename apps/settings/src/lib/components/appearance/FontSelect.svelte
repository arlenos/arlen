<script lang="ts">
  /// Font picker — Typography's Interface + Monospace selectors.
  ///
  /// A thin wrapper around the generic `PopoverSelect` that renders
  /// each option's label in the font it represents, so the list
  /// doubles as a preview. The popover chrome and behaviour live in
  /// `PopoverSelect`; this component just supplies the per-option
  /// render snippet.

  import {
    PopoverSelect,
    type PopoverSelectOption,
  } from "$lib/components/ui/popover-select";

  let {
    value,
    options,
    ariaLabel,
    onchange,
  }: {
    value: string;
    options: PopoverSelectOption[];
    ariaLabel?: string;
    onchange: (value: string) => void;
  } = $props();
</script>

<PopoverSelect
  {value}
  {options}
  {ariaLabel}
  {onchange}
  width="200px"
>
  {#snippet renderLabel(opt: PopoverSelectOption)}
    <span style="font-family: '{opt.value}', ui-sans-serif, monospace;">
      {opt.label}
    </span>
  {/snippet}
</PopoverSelect>
