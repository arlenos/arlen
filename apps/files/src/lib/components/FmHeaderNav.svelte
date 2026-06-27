<script lang="ts">
  /// The headerbar's location half: back/forward on the focused pane,
  /// then the breadcrumb as the flexible middle (the crumb's parent
  /// entries cover going up a folder). Under the shell the topbar
  /// carries the path, so the crumb stays hidden and only Ctrl+L's
  /// editable field takes the middle while it is open. Fragment-rooted:
  /// both pieces sit directly in the header flex row.
  import { ArrowLeft, ArrowRight } from "lucide-svelte";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Breadcrumb, type BrowserState } from "@arlen/ui-kit/components/browser";
  import { placeGroups } from "$lib/stores/places";
  import { locationLabel } from "$lib/locations";

  let {
    controller,
    homePath,
    showCrumb = true,
    pathEditing = $bindable(false),
  }: {
    controller: BrowserState;
    homePath?: string;
    /// False under the shell: the topbar shows the path.
    showCrumb?: boolean;
    /// Bindable: the layout's Ctrl+L flips it.
    pathEditing?: boolean;
  } = $props();

  const path = $derived(controller.path);
  const canBack = $derived(controller.canBack);
  const canForward = $derived(controller.canForward);
  // At a virtual location the breadcrumb renders this as the name crumb.
  const label = $derived(locationLabel($path, $placeGroups));
</script>

<div class="hn-buttons">
  <IconAction
    label="Back"
    size="control"
    disabled={!$canBack}
    onclick={() => controller.back()}
  >
    <ArrowLeft size={15} strokeWidth={1.75} />
  </IconAction>
  <IconAction
    label="Forward"
    size="control"
    disabled={!$canForward}
    onclick={() => controller.forward()}
  >
    <ArrowRight size={15} strokeWidth={1.75} />
  </IconAction>
</div>

{#if showCrumb || pathEditing}
  <div class="hn-crumb">
    <Breadcrumb
      path={$path}
      {homePath}
      locationLabel={label}
      bind:editing={pathEditing}
      onnavigate={(p) => controller.navigate(p)}
    />
  </div>
{/if}

<style>
  .hn-buttons {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .hn-crumb {
    flex: 1;
    min-width: 0;
    margin-left: 4px;
  }
</style>
