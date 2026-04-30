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
  import { Toaster } from "svelte-sonner";
  import { toastConfig, initToastConfig } from "$lib/stores/toastConfig.js";
  import { initToastBridge } from "$lib/stores/toastBridge.js";

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

<slot />
<ContextMenu />
<TabBar />
<Indicator />
<ZoomToolbar />
<WindowHeader />
<BluetoothPairingDialog />
<Toaster
  position={$toastConfig.position}
  richColors
  expand={false}
  closeButton
  theme="dark"
  offset={44}
  toastOptions={{
    style: `width: ${$toastConfig.width}px;`,
    class: `lunaris-toast lunaris-toast-anim-${$toastConfig.animation}`,
  }}
/>
