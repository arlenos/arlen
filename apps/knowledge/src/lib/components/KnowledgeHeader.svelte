<script lang="ts">
  /// The header bar on the files canon: h-10 inside the inset, trigger first,
  /// the PLACE in the middle (never the app name - that lives in the sidebar's
  /// caps label), the graph search and the window controls at the end. Drag
  /// goes through an explicit `startDragging()` (the drag attribute is
  /// unreliable on Wayland in Tauri v2), guarded so vite still renders.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { SidebarTrigger } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { query, clearSearch } from "$lib/stores/search";
  import { t } from "$lib/i18n/messages";

  let { placeLabel }: { placeLabel: string } = $props();

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      clearSearch();
      (e.currentTarget as HTMLInputElement).blur();
    }
  }

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }

  async function startDrag(e: PointerEvent): Promise<void> {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      // No Tauri runtime under vite: the header is a static bar.
    }
  }

  async function toggleMax(e: MouseEvent): Promise<void> {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      // vite: no-op.
    }
  }
</script>

<!-- The header is a drag surface (a non-keyboard pointer interaction); its actual
     controls are the accessible buttons inside it, so the static-interaction
     lint is a false positive here. Same treatment as the files layout. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background px-2"
  onpointerdown={startDrag}
  ondblclick={toggleMax}
>
  <SidebarTrigger class="-ml-1" />
  <Separator orientation="vertical" class="me-1 h-4" />
  <span class="select-none truncate text-sm font-medium text-foreground">{placeLabel}</span>
  <span class="flex-1"></span>
  <!-- The one search entry (Tim's placement call): typing hands the content
       area to the search surface; Esc returns to the place. -->
  <div class="kn-search">
    <SearchField
      bind:value={$query}
      onkeydown={onSearchKey}
      placeholder={$t("k.se.placeholder")}
      aria-label={$t("k.se.aria")}
    />
  </div>
  <WindowButtons />
</header>

<style>
  /* The global search: the shared SearchField in the chrome, sized here. */
  .kn-search {
    width: 15rem;
    margin-inline-end: 0.25rem;
  }
</style>
