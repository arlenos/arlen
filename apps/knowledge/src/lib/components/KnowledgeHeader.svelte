<script lang="ts">
  /// The app titlebar: the drag region + window controls, matching the sibling
  /// apps (settings `SiteHeader`). Window drag goes through an explicit
  /// `startDragging()` (the `data-tauri-drag-region` attribute is unreliable on
  /// Wayland in Tauri v2), and every window call is guarded so the app still
  /// renders under vite, where there is no Tauri runtime.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Search } from "lucide-svelte";
  import { query, clearSearch } from "$lib/stores/search";
  import { t } from "$lib/i18n/messages";

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
      (await w.isMaximized()) ? await w.unmaximize() : await w.maximize();
    } catch {
      // vite: no-op.
    }
  }
</script>

<!-- The header is a drag surface (a non-keyboard pointer interaction); its actual
     controls are the accessible WindowButtons inside it, so the static-interaction
     lint is a false positive here. Same treatment as the meetings layout. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="kn-header" onpointerdown={startDrag} ondblclick={toggleMax}>
  <span class="kn-header-title">{$t("k.title")}</span>
  <span class="kn-header-spacer"></span>
  <!-- The one search entry (Tim's placement call): typing hands the content
       area to the search surface; Esc returns to the place. -->
  <div class="kn-search">
    <Search size={13} strokeWidth={2} class="kn-search-icon" />
    <input
      type="text"
      class="kn-search-input"
      bind:value={$query}
      onkeydown={onSearchKey}
      placeholder={$t("k.se.placeholder")}
      aria-label={$t("k.se.aria")}
    />
  </div>
  <WindowButtons />
</header>

<style>
  .kn-header {
    display: flex;
    align-items: center;
    height: 2.75rem;
    flex-shrink: 0;
    padding: 0 0.35rem 0 0.9rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    user-select: none;
    -webkit-user-select: none;
  }
  .kn-header-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .kn-header-spacer {
    flex: 1;
  }

  /* The global search: a quiet field in the chrome, the Settings search's
     register (subtle fill, input radius, focus firms the border). */
  .kn-search {
    position: relative;
    width: 15rem;
    margin-inline-end: 0.5rem;
  }
  .kn-search :global(.kn-search-icon) {
    position: absolute;
    left: 0.55rem;
    top: 50%;
    transform: translateY(-50%);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    pointer-events: none;
  }
  .kn-search-input {
    width: 100%;
    height: 1.75rem;
    padding: 0 0.6rem 0 1.7rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    font-size: var(--text-xs);
    color: var(--color-fg-primary);
    outline: none;
  }
  .kn-search-input:focus {
    border-color: color-mix(in srgb, var(--color-accent, #6aa9e0) 55%, transparent);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .kn-search-input::placeholder {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
