<script lang="ts">
  /// The app titlebar: drag region + window controls, matching the sibling apps.
  /// Window calls are guarded so the app renders under vite.
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
      // No Tauri runtime under vite.
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

<header class="st-header" onpointerdown={startDrag} ondblclick={toggleMax}>
  <span class="st-header-title">{$t("st.title")}</span>
  <span class="st-header-spacer"></span>
  <WindowButtons />
</header>

<style>
  .st-header {
    display: flex;
    align-items: center;
    height: 2.75rem;
    flex-shrink: 0;
    padding: 0 0.35rem 0 0.9rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    user-select: none;
    -webkit-user-select: none;
  }
  .st-header-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .st-header-spacer {
    flex: 1;
  }
</style>
