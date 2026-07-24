<script lang="ts">
  /// The app titlebar: the drag region + window controls, matching the sibling
  /// apps (settings `SiteHeader`). Window drag goes through an explicit
  /// `startDragging()` (the `data-tauri-drag-region` attribute is unreliable on
  /// Wayland in Tauri v2), and every window call is guarded so the app still
  /// renders under vite, where there is no Tauri runtime.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "$lib/i18n/messages";

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

<header class="kn-header" onpointerdown={startDrag} ondblclick={toggleMax}>
  <span class="kn-header-title">{$t("k.title")}</span>
  <span class="kn-header-spacer"></span>
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
</style>
