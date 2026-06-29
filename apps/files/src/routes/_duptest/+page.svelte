<script lang="ts">
  /// Headless render harness for the duplicate finder. UI-AFFORDANCE
  /// verification ONLY, NOT a behaviour claim. Seeds the review stores with
  /// mocked groups (the BLAKE3 scan is the coder's `files_find_duplicates`) and
  /// renders the real FmDuplicates in its results state - the grouped copies, the
  /// keep/trash controls, the keep-one guard, the reclaimable summary, the
  /// confirm. The scanning + clean states are reached by flipping the stores.
  /// Dev route only.
  import { onMount } from "svelte";
  import FmDuplicates from "$lib/components/FmDuplicates.svelte";
  import {
    duplicateGroups,
    duplicatesScanning,
    duplicatesScope,
    keepNewest,
    type DupGroup,
  } from "$lib/stores/duplicates";

  const GROUPS: DupGroup[] = [
    {
      hash: "h1",
      files: [
        { path: "/home/tim/Pictures/2024/inn-sunset.jpg", name: "inn-sunset.jpg", size: 6_300_000, modified_unix: 1_786_000_000 },
        { path: "/home/tim/Pictures/downloads/inn-sunset (1).jpg", name: "inn-sunset (1).jpg", size: 6_300_000, modified_unix: 1_785_000_000 },
      ],
    },
    {
      hash: "h2",
      files: [
        { path: "/home/tim/Pictures/berg.png", name: "berg.png", size: 4_100_000, modified_unix: 1_782_000_000 },
        { path: "/home/tim/Pictures/old/berg-copy.png", name: "berg-copy.png", size: 4_100_000, modified_unix: 1_779_000_000 },
        { path: "/home/tim/Downloads/berg(1).png", name: "berg(1).png", size: 4_100_000, modified_unix: 1_779_000_000 },
      ],
    },
    {
      hash: "h3",
      files: [
        { path: "/home/tim/Documents/notes.md", name: "notes.md", size: 12_400, modified_unix: 1_783_000_000 },
        { path: "/home/tim/Documents/backup/notes.md", name: "notes.md", size: 12_400, modified_unix: 1_781_000_000 },
      ],
    },
  ];

  // ?state=scanning|clean switches the view; default is the results state.
  let ready = $state(false);
  onMount(() => {
    duplicatesScope.set("/home/tim/Pictures");
    const state = new URLSearchParams(window.location.search).get("state");
    if (state === "scanning") {
      duplicatesScanning.set(true);
      duplicateGroups.set(null);
    } else if (state === "clean") {
      duplicateGroups.set([]);
    } else {
      duplicateGroups.set(GROUPS);
      keepNewest();
    }
    ready = true;
  });
</script>

<div class="harness">
  {#if ready}
    <FmDuplicates ontrash={() => {}} />
  {/if}
</div>

<style>
  .harness {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--background);
    color: var(--foreground);
  }
</style>
