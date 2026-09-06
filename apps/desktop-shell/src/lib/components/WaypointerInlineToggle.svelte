<script lang="ts">
  /// Compact inline toggle for Waypointer settings results.
  /// Uses the unified Switch component with size="sm".

  import Switch from "@arlen/ui-kit/components/ui/switch/switch.svelte";

  let {
    checked,
    onchange,
    label,
  }: {
    checked: boolean;
    onchange: (value: boolean) => void;
    /// What this toggle governs, for a reader who cannot see the row it sits in.
    /// Required rather than optional: a switch with no name is announced as
    /// "switch", and this one sits in a list of them.
    label: string;
  } = $props();
</script>

<!-- stopPropagation so clicking the toggle doesn't trigger
     the CommandItem's onSelect (which opens the Settings app). -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div onclick={(e) => e.stopPropagation()}>
  <Switch ariaLabel={label} value={checked} {onchange} size="sm" />
</div>
