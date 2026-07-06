<script lang="ts">
  import { onMount } from "svelte";
  import { initTheme } from "$lib/theme";
  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import "../app.css";

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && $activePopover !== null) {
      e.preventDefault();
      closePopover();
    }
  }

  /// Suppress the webview's native "Back / Forward / Reload / Inspect"
  /// menu. The shell renders its own context menus (compositor-driven
  /// window menus, row-level SNI/menu entries, etc.) — never show the
  /// browser one. Opt-out via `data-allow-browser-context` attribute.
  function suppressBrowserContextMenu(e: MouseEvent): void {
    if ((e.target as HTMLElement | null)?.closest?.(
      "[data-allow-browser-context]"
    )) {
      return;
    }
    e.preventDefault();
  }
  import { initWindowListeners } from "$lib/stores/windows";
  import { initContextMenuListeners } from "$lib/stores/contextMenu.js";
  import { initNotifications } from "$lib/stores/notifications.js";
  import { initWorkspaceListeners } from "$lib/stores/workspaces.js";
  import { initMenuListeners } from "$lib/stores/menus.js";
  import { initTabBarListeners } from "$lib/stores/tabBars";
  import { initIndicatorListeners } from "$lib/stores/indicators";
  import { initZoomListeners } from "$lib/stores/zoom";
  import { initWindowHeaderListeners } from "$lib/stores/windowHeaders";
  import { initProjects } from "$lib/stores/projects.js";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import Indicator from "$lib/components/Indicator.svelte";
  import ZoomToolbar from "$lib/components/ZoomToolbar.svelte";
  import WindowHeader from "$lib/components/WindowHeader.svelte";
  import BluetoothPairingDialog from "$lib/components/BluetoothPairingDialog.svelte";
  import AuthorizationDialog from "$lib/components/AuthorizationDialog.svelte";
  import ConsentDialog from "$lib/components/ConsentDialog.svelte";
  import AmbientOverlay from "$lib/components/AmbientOverlay.svelte";
  import { Toaster } from "svelte-sonner";
  import { toastConfig, initToastConfig } from "$lib/stores/toastConfig.js";
  import { initToastBridge } from "$lib/stores/toastBridge.js";
  import { initToolbarStore } from "$lib/stores/toolbarStore";
  import { initAppStateStores } from "$lib/stores/appStateStores";

  /// Top of the QS / Notifications popover panels. Matches their
  /// CSS `top: 40px` so the math here stays in lock-step with where
  /// the panels actually land.
  const PANEL_TOP = 40;
  /// Distance (px) from the top of the screen at which toasts begin
  /// when no panel is open. Topbar (36px) + 8px breathing room.
  const TOAST_BASE_OFFSET = 44;
  /// Gap between an open panel's bottom edge and the toast stack
  /// below it. 24px gives a clear visual break so toasts don't read
  /// as part of the panel.
  const TOAST_PANEL_GAP = 24;

  /// Live-measured height of the open right-column panel
  /// (QuickSettingsPanel or NotificationsPopover). Drives the
  /// Toaster `offset` so toasts always land BELOW whichever panel
  /// is open instead of overlapping it.
  ///
  /// One-shot RAF measurement was insufficient — async tile
  /// content (KnowledgeTile chart loads after a graph-query round-
  /// trip, NotificationPanel grows when notifications stream in)
  /// makes the panel grow tens of pixels AFTER first measurement,
  /// leaving the toast stack ~2cm too high. ResizeObserver tracks
  /// the live height and updates `panelHeight` on every layout
  /// shift, so the toast stays glued to the panel's bottom edge
  /// even when the panel keeps growing. Falls back to
  /// `offsetHeight` (transform-independent) so the popover's
  /// scale-in animation doesn't briefly under-measure.
  let panelHeight = $state(0);

  $effect(() => {
    const id = $activePopover;
    if (id !== "quick-settings" && id !== "notifications") {
      panelHeight = 0;
      return;
    }

    const sel = id === "quick-settings" ? ".qs-panel" : ".np-popover";
    let observer: ResizeObserver | null = null;
    let raf: number | null = null;

    function attach() {
      const el = document.querySelector<HTMLElement>(sel);
      if (!el) {
        // Panel not yet in DOM — try again next frame.
        raf = requestAnimationFrame(attach);
        return;
      }
      // Initial measurement uses offsetHeight (transform-
      // independent) so the in-progress popover-in animation
      // doesn't briefly report a scale(0.98) box.
      panelHeight = el.offsetHeight;
      observer = new ResizeObserver((entries) => {
        for (const entry of entries) {
          const target = entry.target as HTMLElement;
          panelHeight = target.offsetHeight;
        }
      });
      observer.observe(el);
    }
    attach();

    return () => {
      if (raf !== null) cancelAnimationFrame(raf);
      observer?.disconnect();
    };
  });

  const toasterOffset = $derived(
    panelHeight > 0
      ? PANEL_TOP + panelHeight + TOAST_PANEL_GAP
      : TOAST_BASE_OFFSET,
  );

  onMount(() => {
    // Every store init now returns a disposer. Collecting them lets
    // onMount's return closure tear down every Tauri listener on
    // unmount, preventing the "every HMR adds another listener" leak
    // that was making the shell slower with time.
    const disposers: Array<() => void> = [
      initWindowListeners(),
      initContextMenuListeners(),
      initNotifications(),
      initWorkspaceListeners(),
      initMenuListeners(),
      initTabBarListeners(),
      initIndicatorListeners(),
      initZoomListeners(),
      initWindowHeaderListeners(),
      initProjects(),
      initToastConfig(),
      initToastBridge(),
      initToolbarStore(),
      initAppStateStores(),
    ];

    // Initialize theme system (loads appearance.toml, injects CSS vars,
    // subscribes to live theme-changed events from Rust). Its internal
    // `listen()` lives for the lifetime of the page — it has no init/
    // dispose pair because the theme store is module-scoped state.
    initTheme().catch(() => {});

    document.addEventListener("contextmenu", suppressBrowserContextMenu);
    return () => {
      document.removeEventListener("contextmenu", suppressBrowserContextMenu);
      for (const dispose of disposers) dispose();
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<AmbientOverlay />
<slot />
<ContextMenu />
<TabBar />
<Indicator />
<ZoomToolbar />
<WindowHeader />
<BluetoothPairingDialog />
<AuthorizationDialog />
<ConsentDialog />
<!-- Per-side offsets: the vertical offset tracks the open panel
     (panel-avoidance math above); the right edge is the shell's own
     8px. Setting both here keeps the geometry where the element is
     configured instead of fighting the library CSS. -->
<Toaster
  position={$toastConfig.position}
  richColors
  expand={false}
  closeButton
  theme="dark"
  offset={{ top: toasterOffset, right: 8 }}
  toastOptions={{
    style: `width: ${$toastConfig.width}px;`,
    class: `arlen-toast arlen-toast-anim-${$toastConfig.animation}`,
  }}
/>
