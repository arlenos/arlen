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
<!-- A floor under the place name and a header that can grow, the same fix the
     calendar, the reader and mail needed for the same reason: a `truncate` with
     nothing under it does not truncate when the row runs out, it collapses. Here
     the search field takes a fixed share beside it, so at 720 "Suchen" was two
     pixels short of itself and "Bibliothek" five. `min-h-10` with no `h-10`
     above it lets the browser say when a second row is due; a breakpoint would
     be a guess about how long a place is called in the next language. -->
<header
  class="flex min-h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border bg-background px-2"
  onpointerdown={startDrag}
  ondblclick={toggleMax}
>
  <SidebarTrigger class="-ml-1" />
  <!-- `h-4!`: the kit sets `data-[orientation=vertical]:h-full`, an attribute
       selector a plain `h-4` cannot outrank, and a full-height rule in a header
       that wraps is a percentage of a height still being computed. -->
  <Separator orientation="vertical" class="me-1 h-4!" />
  <span class="min-w-[10ch] select-none truncate text-sm font-medium text-foreground">{placeLabel}</span>
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
