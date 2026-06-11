<script lang="ts">
  /// The navigation toolbar: back/forward/up on the controller, the
  /// breadcrumb as the single location display (Ctrl+L turns it into
  /// the editable path field), and the hidden-files toggle. View
  /// switch and search join with their increments.
  import { ArrowLeft, ArrowRight, ArrowUp, Eye, EyeOff } from "lucide-svelte";
  import { Toolbar } from "@arlen/ui-kit/components/ui/toolbar";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Breadcrumb, type BrowserState } from "@arlen/ui-kit/components/browser";

  let {
    controller,
    homePath,
    pathEditing = $bindable(false),
  }: {
    controller: BrowserState;
    homePath?: string;
    /// Bindable: the layout's Ctrl+L flips it.
    pathEditing?: boolean;
  } = $props();

  const path = $derived(controller.path);
  const canBack = $derived(controller.canBack);
  const canForward = $derived(controller.canForward);
  const canUp = $derived(controller.canUp);
  const showHidden = $derived(controller.showHidden);
</script>

<Toolbar class="fm-toolbar">
  {#snippet start()}
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
    <IconAction
      label="Up one folder"
      size="control"
      disabled={!$canUp}
      onclick={() => controller.up()}
    >
      <ArrowUp size={15} strokeWidth={1.75} />
    </IconAction>
    <div class="fm-crumb">
      <Breadcrumb
        path={$path}
        {homePath}
        bind:editing={pathEditing}
        onnavigate={(p) => controller.navigate(p)}
      />
    </div>
  {/snippet}
  {#snippet end()}
    <IconAction
      label={$showHidden ? "Hide hidden files" : "Show hidden files"}
      size="control"
      active={$showHidden}
      onclick={() => controller.setShowHidden(!$showHidden)}
    >
      {#if $showHidden}
        <Eye size={15} strokeWidth={1.75} />
      {:else}
        <EyeOff size={15} strokeWidth={1.75} />
      {/if}
    </IconAction>
  {/snippet}
</Toolbar>

<style>
  .fm-crumb {
    flex: 1;
    min-width: 0;
    margin-left: 4px;
  }
</style>
