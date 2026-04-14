<script lang="ts">
  /// Top bar: sidebar trigger + breadcrumbs + window controls.
  ///
  /// The header is the tauri drag region, so the whole empty area between
  /// the breadcrumb and the window controls can be used to drag the window.
  import { breadcrumbs } from "$lib/stores/navigation";
  import { SidebarTrigger } from "$lib/components/ui/sidebar";
  import Separator from "$lib/components/ui/separator.svelte";
  import WindowControls from "./WindowControls.svelte";
  import { ChevronRight } from "lucide-svelte";
</script>

<header
  data-tauri-drag-region
  class="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-background pl-2 pr-2"
>
  <div class="flex items-center gap-2">
    <SidebarTrigger class="-ml-1" />
    <Separator orientation="vertical" class="mr-1 h-4" />
    <nav
      class="flex items-center gap-1.5 text-sm text-muted-foreground"
      aria-label="Breadcrumb"
    >
      {#each $breadcrumbs as segment, i (segment + i)}
        {#if i > 0}
          <ChevronRight size={14} class="opacity-50" />
        {/if}
        <span
          class={i === $breadcrumbs.length - 1
            ? "font-medium text-foreground"
            : ""}
        >
          {segment}
        </span>
      {/each}
    </nav>
  </div>

  <div class="flex-1" data-tauri-drag-region></div>

  <WindowControls />
</header>
