<script lang="ts">
  /// The picker route is a thin wrapper over `PickerView`, which holds
  /// the whole surface. Splitting the view out lets the dev mock route
  /// (`/_pickertest`) render the same component against a seeded Tauri
  /// IPC mock for the screenshot loop.
  import PickerView from "$lib/components/PickerView.svelte";
</script>

<PickerView />
