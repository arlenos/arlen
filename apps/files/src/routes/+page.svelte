<script lang="ts">
  /// The browser surface: hosts the shared FileBrowser on one
  /// controller. Tabs, toolbar and sidebar land with the shell
  /// increment; this page already navigates, sorts and selects.
  import {
    createBrowserState,
    FileBrowser,
  } from "@arlen/ui-kit/components/browser";
  import { fmAdapter, openPath } from "$lib/adapter";

  const controller = createBrowserState(fmAdapter, { initial: "/home/tim" });
</script>

<FileBrowser
  {controller}
  onactivate={(entry, path) => {
    if (entry.kind !== "directory") void openPath(path);
  }}
/>
