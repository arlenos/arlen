<script lang="ts">
  /// Focus Mode settings page.
  ///
  /// Skeleton from Sprint A — fields wired in Sprint C. Configures
  /// `shell.toml [focus]` (default suppressed apps, show-project-name)
  /// and `graph.toml [project]` (auto-promote threshold, watch
  /// directories, signal weights). Uses `AppPicker`,
  /// `DirectoryPicker`, `AddRemoveList` from Sprint A.

  import Placeholder from "$lib/components/settings/Placeholder.svelte";
</script>

<Placeholder
  title="Focus Mode"
  description="Project detection, default suppressed apps, watch directories. Wired in Sprint C."
/>
