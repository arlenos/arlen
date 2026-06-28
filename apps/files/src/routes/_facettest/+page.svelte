<script lang="ts">
  /// Headless render harness for the faceted KG filter bar. UI-AFFORDANCE
  /// verification ONLY, NOT a behaviour claim. Mocks the daemon over Tauri IPC
  /// (only when no Tauri runtime is present, so it never hijacks the real app):
  /// the project + touched option reads. Renders the real FmFacetBar with a
  /// populated selection (so the active chips and dropdown checks show) over a
  /// stand-in result listing, so the whole composition reads. The empty state,
  /// an open dropdown and the save dialog are reached by driving the controls.
  /// The live KG query is the coder's `files_list_location` for `facet:` keys;
  /// this proves the controls, not the data. Not shipped in any nav; a dev route.
  import { onMount } from "svelte";
  import { tauriAvailable } from "$lib/tauri";
  import FmFacetBar from "$lib/components/FmFacetBar.svelte";
  import { selectedFacets, savedFolders, loadFacetOptions } from "$lib/stores/facets";

  // The stand-in faceted result, what the listing shows under the bar.
  const RESULT = [
    { name: "thesis-intro.md", where: "Thesis writeup", modified: "4 days ago" },
    { name: "inn-sunset.jpg", where: "Reading list", modified: "6 days ago" },
    { name: "lit-review.md", where: "Thesis writeup", modified: "yesterday" },
    { name: "diagram-final.png", where: "Thesis writeup", modified: "2 days ago" },
  ];

  let ready = $state(false);
  onMount(async () => {
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd) => {
        if (cmd === "files_projects")
          return [
            { id: "p-thesis", name: "Thesis writeup", path: "/home/tim/thesis" },
            { id: "p-reading", name: "Reading list", path: "/home/tim/reading" },
            { id: "p-os", name: "Arlen OS", path: "/home/tim/arlen" },
          ];
        if (cmd === "files_touched_apps")
          return [
            { id: "files", label: "Files", count: 42 },
            { id: "viewer", label: "Image viewer", count: 11 },
            { id: "terminal", label: "Terminal", count: 7 },
          ];
        return [];
      });
    }
    await loadFacetOptions();
    // A populated filter: two projects, two types, a recency cutoff.
    selectedFacets.set({
      project: new Set(["p-thesis", "p-reading"]),
      type: new Set(["document", "image"]),
      time: new Set(["week"]),
      touched: new Set(),
    });
    savedFolders.set([
      { id: "sf-1", name: "Thesis images", location: "facet:project=p-thesis;type=image" },
    ]);
    ready = true;
  });
</script>

<div class="harness">
  {#if ready}
    <section class="host">
      <h2>Active filter + result</h2>
      <div class="frame">
        <FmFacetBar basePath="/home/tim" onnavigate={() => {}} />
        <div class="listing">
          <div class="lhead">
            <span>Name</span><span>Location</span><span>Modified</span>
          </div>
          {#each RESULT as r (r.name)}
            <div class="lrow">
              <span class="lname">{r.name}</span>
              <span class="lwhere">{r.where}</span>
              <span class="lmod">{r.modified}</span>
            </div>
          {/each}
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .harness {
    display: flex;
    flex-direction: column;
    gap: 28px;
    padding: 24px;
    min-height: 100vh;
    background: var(--background);
  }
  .host {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 720px;
  }
  h2 {
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .frame {
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    border-radius: var(--radius-card, 10px);
    overflow: hidden;
    background: var(--background);
  }
  .listing {
    font-size: 0.8125rem;
  }
  .lhead,
  .lrow {
    display: grid;
    grid-template-columns: 1fr 1fr 120px;
    gap: 12px;
    padding: 6px 14px;
    align-items: center;
  }
  .lhead {
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-size: 0.6875rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .lrow + .lrow {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .lname {
    color: var(--foreground);
  }
  .lwhere,
  .lmod {
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
</style>
