<script lang="ts">
  /// The headerbar's location half: back/forward on the focused pane,
  /// then the breadcrumb as the flexible middle (the crumb's parent
  /// entries cover going up a folder). Under the shell the topbar
  /// carries the path, so the crumb stays hidden and only Ctrl+L's
  /// editable field takes the middle while it is open. Fragment-rooted:
  /// both pieces sit directly in the header flex row.
  import { ArrowLeft, ArrowRight } from "lucide-svelte";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Breadcrumb, type BrowserState } from "@arlen/ui-kit/components/browser";
  import { placeGroups } from "$lib/stores/places";
  import { locationLabel } from "$lib/locations";
  import { AS_OF_OPTIONS, viewAsOfChoice } from "$lib/asof";
  import { t } from "$lib/i18n/messages";

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

  // Whole-listing time-travel is meaningful only on a project location (the
  // bitemporal membership slice); the control appears there and re-lists.
  const isProject = $derived($path.startsWith("project:"));
  function onAsOf(v: string): void {
    viewAsOfChoice.set(v);
    void controller.refresh();
  }
</script>

<div class="hn-buttons">
  <IconAction
    label={$t("f.nav.back")}
    size="control"
    disabled={!$canBack}
    onclick={() => controller.back()}
  >
    <ArrowLeft size={15} strokeWidth={1.75} />
  </IconAction>
  <IconAction
    label={$t("f.nav.forward")}
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

{#if isProject}
  <div class="hn-asof">
    <PopoverSelect
      value={$viewAsOfChoice}
      options={AS_OF_OPTIONS}
      ariaLabel={$t("f.nav.asOfAria")}
      width="9.5rem"
      onchange={onAsOf}
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
    margin-inline-start: 4px;
  }
  .hn-asof {
    flex-shrink: 0;
    margin-inline-start: 4px;
  }
</style>
