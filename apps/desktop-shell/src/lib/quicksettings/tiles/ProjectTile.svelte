<script lang="ts">
  /// QS tile: project context (Focus Mode entry point).
  ///
  /// 1×1 tile paired with the Knowledge tile in row 1. Two visual
  /// modes:
  ///   - Focused:    open-folder icon, project name as label,
  ///                 "Focus Mode" subtitle, accent-tinted background.
  ///   - Unfocused:  FolderPlus icon (CTA), "Start a project" label,
  ///                 dimmed via reduced foreground so the empty
  ///                 state reads as a prompt instead of a status row.
  ///
  /// Click opens the Waypointer with `p:` prefix in either state so
  /// the user can pick / switch projects. The full project root path
  /// is dropped at 1×1 — there isn't enough horizontal room. Users
  /// who need the path can right-click → resize the tile to 2×1 in
  /// Settings (the path-text returns automatically).
  import { BaseTile } from "@arlen/ui-kit/components/quicksettings";
  import { t } from "$lib/i18n/messages";
  import { FolderOpen, FolderPlus } from "lucide-svelte";
  import { focusState, focusedProject } from "$lib/stores/projects.js";
  import { closePopover } from "$lib/stores/activePopover.js";
  import { shellAction } from "$lib/shellAction";

  function openWaypointerWithProjectPrefix() {
    closePopover();
    // The top bar's project switcher is the same press through another door, and
    // it swallowed the same answer. Both say it now.
    void shellAction("set_query_and_show", { query: "p:", mode: "" }, "sh.toast.launcherClosed");
  }
</script>

<div class="project-tile-wrap">
  <BaseTile
    label={$focusedProject?.name ?? $t("sh.tile.startProject")}
    statusText={$focusState.projectId ? $t("sh.tile.focusMode") : $t("sh.tile.tapToPick")}
    active={!!$focusState.projectId}
    onclick={openWaypointerWithProjectPrefix}
  >
    {#snippet icon()}
      {#if $focusState.projectId}
        <FolderOpen size={16} strokeWidth={1.75} />
      {:else}
        <FolderPlus size={16} strokeWidth={1.75} />
      {/if}
    {/snippet}
  </BaseTile>
</div>

<style>
  /* NO FADE ON THE NO-PROJECT STATE, and it was here until 6 September. The kit
     draws this tile's status line at 55% of the foreground, which measures 6.04:1
     over the panel; a wrapper opacity of 0.7 multiplied that to an effective
     38.5% and about 3.4:1, under the 4.5 small text needs - so the state the fade
     was FOR was the state whose words could not be read. axe found it the first
     time any sweep opened this panel.
     Nothing is lost by dropping it: the tile already says which state it is in
     three other ways - a different icon (FolderPlus against FolderOpen), a
     different label, and the kit's own `active` styling. */
  .project-tile-wrap {
    width: 100%;
  }
  .project-tile-wrap :global(.qs-tile) {
    width: 100%;
  }
</style>
