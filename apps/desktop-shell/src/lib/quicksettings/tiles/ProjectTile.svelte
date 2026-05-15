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
  import { BaseTile } from "@lunaris/ui-kit/components/quicksettings";
  import { FolderOpen, FolderPlus } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { focusState, focusedProject } from "$lib/stores/projects.js";
  import { closePopover } from "$lib/stores/activePopover.js";

  function openWaypointerWithProjectPrefix() {
    closePopover();
    invoke("set_query_and_show", { query: "p:", mode: "" }).catch(() => {});
  }
</script>

<div class="project-tile-wrap" class:dim={!$focusState.projectId}>
  <BaseTile
    label={$focusedProject?.name ?? "Start a project"}
    statusText={$focusState.projectId ? "Focus Mode" : "Tap to pick"}
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
  .project-tile-wrap {
    width: 100%;
    transition: opacity 100ms ease;
  }
  .project-tile-wrap.dim {
    opacity: 0.7;
  }
  .project-tile-wrap.dim:hover {
    opacity: 1;
  }
  .project-tile-wrap :global(.qs-tile) {
    width: 100%;
  }
</style>
