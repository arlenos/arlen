<script lang="ts">
  /// Compact inline toggle for Waypointer settings results.
  /// Uses the unified Switch component with size="sm".

  import Switch from "$lib/components/ui/switch/switch.svelte";

  let {
    checked,
    onchange,
  }: {
    checked: boolean;
    onchange: (value: boolean) => void;
  } = $props();
</script>

<!-- stopPropagation so clicking the toggle doesn't trigger
     the CommandItem's onSelect (which opens the Settings app). -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div onclick={(e) => e.stopPropagation()}>
  <Switch value={checked} {onchange} size="sm" />
</div>
