<script lang="ts">
  /// Headless render harness for the FM sidebar, the `_facettest` pattern.
  ///
  /// The sidebar carries two sentences that only appear when something went
  /// wrong, and neither can be reached from the running app on purpose: the
  /// places read has to fail, or a Smart Folder write has to be refused. Under
  /// plain vite the first is easy (there is no Tauri) and the second never fires,
  /// because with no places and no folders the groups do not render at all.
  ///
  /// `?state=unsaved` puts a folder in the list and refuses its write, which is
  /// what a person sees after defining one on a machine whose config directory is
  /// not writable: the folder is on screen and will not survive a restart.
  /// `?locale=de` renders it in German. Not in any nav; a dev route.
  ///
  /// THE SIDEBAR ON SCREEN IS THE LAYOUT'S, not one this page mounts. A page in
  /// this app is a child of the root layout and cannot escape it, so a second
  /// `FmSidebar` here put two of them on the page - two `Orte` landmarks a reader
  /// cannot tell apart, which is what axe reported as `landmark-unique`, and a
  /// picture of a rail that does not exist in the running app. This route seeds
  /// the stores and lets the real one render them.
  import { onMount } from "svelte";
  import { savedFolders, foldersUnsaved } from "$lib/stores/facets";
  import { locale } from "@arlen/ui-kit/i18n";

  const params =
    typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  if (params?.get("locale")) locale.set(params.get("locale") as string);
  const unsaved = params?.get("state") === "unsaved";

  onMount(() => {
    if (unsaved) {
      savedFolders.set([
        { id: "sf-1", name: "Thesis images", location: "facet:project=p-thesis;type=image" },
      ]);
      // Set directly rather than by tripping the persist subscription: that only
      // runs after `loadSmartFolders`, which needs a host. The state under test is
      // the sentence, not the path that reaches it.
      foldersUnsaved.set(true);
    }
  });
</script>
