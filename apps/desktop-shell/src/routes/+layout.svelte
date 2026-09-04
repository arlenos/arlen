<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { initTheme } from "$lib/theme";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
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
  import { initConsoleBridge } from "$lib/stores/consoleBridge";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import Indicator from "$lib/components/Indicator.svelte";
  import ZoomToolbar from "$lib/components/ZoomToolbar.svelte";
  import WindowHeader from "$lib/components/WindowHeader.svelte";
  import BluetoothPairingDialog from "$lib/components/BluetoothPairingDialog.svelte";
  import ConsentDialog from "$lib/components/ConsentDialog.svelte";
  import SourcePicker from "$lib/components/SourcePicker.svelte";
  import WindowsFileDialog from "$lib/components/WindowsFileDialog.svelte";
  import PrintDialog from "$lib/components/PrintDialog.svelte";
  import MenuPalette from "$lib/components/MenuPalette.svelte";
  import VoiceHud from "$lib/components/VoiceHud.svelte";
  import AmbientOverlay from "$lib/components/AmbientOverlay.svelte";
  import { Toaster } from "svelte-sonner";
  import { toastConfig, initToastConfig } from "$lib/stores/toastConfig.js";
  import { initToastBridge } from "$lib/stores/toastBridge.js";
  import { watchForPrints } from "$lib/stores/printDialog.js";
  import { watchJobs } from "$lib/stores/jobs.js";
  import { initToolbarStore } from "$lib/stores/toolbarStore";
  import { initAppStateStores } from "$lib/stores/appStateStores";

  // Which window this document is, read straight off the Tauri host object: the
  // label is the only thing needed and this needs no import and no throwing path.
  // Absent host (vite) reads as the main window, so every surface still renders for
  // the screenshot loop.
  const windowLabel =
    (globalThis as Record<string, any>).__TAURI_INTERNALS__?.metadata?.currentWindow
      ?.label ?? "main";
  const isMainWindow = windowLabel === "main";
  /// The consent card renders here and nowhere else. It moved out of the bar
  /// because a layer surface is only granted keyboard focus when it maps, so the
  /// bar's runtime switch to exclusive interactivity never produced focus and
  /// Escape-to-deny never reached the page. The consent window maps exclusive.
  /// The one-surface rule the guard below was written for still holds; it is the
  /// same rule, pointed at a different window.
  const isConsentWindow = windowLabel === "consent";

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
  let { children } = $props();

  let panelHeight = $state(0);

  $effect(() => {
    const id = $activePopover;
    if (id === null) {
      panelHeight = 0;
      return;
    }

    // ANY open panel, not just those two. The list used to be
    // quick-settings and notifications, and every other applet
    // panel measured as zero - so the toast stack started at 44px
    // while the panel starts at 40, and a toast arriving while the
    // sound or network panel was open landed squarely on top of it.
    // The first thing it covers is the strip along the panel's top
    // edge, which is exactly where those panels put "That change
    // did not reach the audio service." Quick settings keeps its own
    // selector; everything else is a ShellPopover and wears
    // `.pop-panel`.
    //
    // This also revives the notifications case, which had stopped
    // working silently: it looked for `.np-popover`, a class that no
    // longer exists anywhere in the tree since that panel moved onto
    // ShellPopover. The selector matched nothing, so `attach` spun on
    // requestAnimationFrame for as long as the panel stayed open and
    // the offset never moved off its base.
    const sel = id === "quick-settings" ? ".qs-panel" : ".pop-panel";
    let observer: ResizeObserver | null = null;
    let raf: number | null = null;

    // A panel takes a frame or two to mount; a selector that has been
    // renamed away takes forever. Bounded so the second case says so
    // once instead of retrying in silence for as long as the panel is
    // open, which is how `.np-popover` went unnoticed.
    let tries = 0;
    function attach() {
      const el = document.querySelector<HTMLElement>(sel);
      if (!el) {
        if (++tries > 30) {
          console.warn(
            `[shell] no element matches ${sel}; toasts will overlap the ${id} panel`,
          );
          return;
        }
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
    // ONE line, once, the first time a pointer is pressed anywhere on the bar.
    //
    // The shell could not answer "did a click reach me at all" from a boot log,
    // and on 21 August that question cost a night: the top bar takes hover and
    // opens no panel on the image, and with no signal here I could not tell a
    // press that never arrived from a press the webview swallowed. Capture
    // phase, so it fires even if something downstream stops propagation, and
    // once, so a normal session pays a single line.
    let sawPointer = false;
    const firstPointer = (e: PointerEvent) => {
      if (sawPointer) return;
      sawPointer = true;
      // The tag alone was not enough: the first run of this said DIV, which
      // names no element in a bar built of buttons. The class and two ancestors
      // are what identify the thing actually taking the press.
      const path: string[] = [];
      let n: Element | null = e.target instanceof Element ? e.target : null;
      while (n && path.length < 3) {
        const cls = typeof n.className === "string" ? n.className.split(" ").slice(0, 2).join(".") : "";
        path.push(n.tagName + (cls ? "." + cls : ""));
        n = n.parentElement;
      }
      // When the press lands on something that covers the whole screen, say WHAT
      // it is. A full-viewport element over the shell swallows every click while
      // the bar underneath still shows hover, which is the exact state that cost
      // a night on 21 August: hover worked, no panel ever opened, and the target
      // logged only as "DIV".
      let covering = "";
      if (e.target instanceof Element) {
        const r = e.target.getBoundingClientRect();
        if (r.width >= window.innerWidth * 0.9 && r.height >= window.innerHeight * 0.9) {
          const slot = e.target.getAttribute("data-slot") ?? "";
          const cls = typeof e.target.className === "string" ? e.target.className : "";
          covering = ` COVERS THE SCREEN slot=${slot || "none"} class=${cls.slice(0, 70)}`;
        }
      }
      invoke("log_frontend", {
        message: `[input] first pointerdown at ${Math.round(e.clientX)},${Math.round(e.clientY)} on ${path.join(" < ") || "nothing"}${covering}`,
      }).catch(() => {});
    };
    window.addEventListener("pointerdown", firstPointer, { capture: true });

    // Every store init now returns a disposer. Collecting them lets
    // onMount's return closure tear down every Tauri listener on
    // unmount, preventing the "every HMR adds another listener" leak
    // that was making the shell slower with time.
    // FIRST, before anything that can raise a refusal. A toast is a frozen
    // string: the bridge words it from the catalog at the moment the event
    // arrives, so a startup failure raised before the catalog is in hand is
    // worded in English and stays English for its whole life on screen. The
    // focus-mode restore in `initProjects` is exactly such a failure. This does
    // not make the race impossible - the read is async and nothing here awaits
    // it - but it starts the local config read before the first init that can
    // fail, which is what decides it in practice.
    void initArlenLocale();

    const disposers: Array<() => void> = [
      // First, so a failure in any init below is logged rather than swallowed.
      initConsoleBridge(),
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

    // Two listeners that resolve to their own disposer rather than returning
    // one, so they cannot go in the array above. The print portal holds a
    // connection open and says when a print is waiting; the jobs feed keeps the
    // Activity zone following work that was already running when it opened.
    // Torn down through the promise, which is why they are kept apart rather
    // than fired and forgotten.
    const asyncListeners = [watchForPrints(), watchJobs()];

    // Initialize theme system (loads appearance.toml, injects CSS vars,
    // subscribes to live theme-changed events from Rust). Its internal
    // `listen()` lives for the lifetime of the page — it has no init/
    // dispose pair because the theme store is module-scoped state.
    initTheme().catch(() => {});

    document.addEventListener("contextmenu", suppressBrowserContextMenu);
    return () => {
      for (const pending of asyncListeners) {
        void pending.then((unlisten) => unlisten?.()).catch(() => {});
      }
      document.removeEventListener("contextmenu", suppressBrowserContextMenu);
      window.removeEventListener("pointerdown", firstPointer, { capture: true });
      for (const dispose of disposers) dispose();
    };
  });
  import { t } from "$lib/i18n/messages";
</script>

<svelte:head>
  <!-- Three windows share this layout, so the title is which one this is:
       a task switcher showing three entries called "Arlen" is no better
       than three with no name at all. -->
  <title>
    {windowLabel === "waypointer"
      ? $t("sh.app.title.waypointer")
      : windowLabel === "consent"
        ? $t("sh.app.title.consent")
        : $t("sh.app.title")}
  </title>
</svelte:head>

<svelte:window onkeydown={handleKeydown} />

<!-- Desktop chrome, and not on the consent surface. `AmbientOverlay` is
     `position: fixed; inset: 0` with a solid ground, so on a fullscreen modal
     window it paints over the desktop instead of letting the card's 50% dim show
     it: measured on the image, the frame was 11.4% non-black with the card up and
     100% the moment it closed, and the top bar was invisible underneath. The rest
     is desktop furniture that a modal has no business drawing. -->
{#if !isConsentWindow}
  <AmbientOverlay />
{/if}
{@render children?.()}
{#if !isConsentWindow}
  <ContextMenu />
  <TabBar />
  <Indicator />
  <ZoomToolbar />
  <WindowHeader />
  <BluetoothPairingDialog />
{/if}
<!-- One window, not every window. There is a single root layout and no route of
     its own for the waypointer, so every shell window - the bar, one extra bar per
     additional output, and the fullscreen waypointer - was mounting the consent
     dialog, polling the broker each second and rendering its own copy of the card.
     A boot showed the same request answered twice, once per window, and the hidden
     window's card then sat in its DOM for good. A system-modal request belongs to
     exactly one surface.
     The sibling modals below have the same shape and are NOT yet narrowed; that is
     a follow-up, kept separate so this boot stays attributable. -->
{#if isConsentWindow}
  <ConsentDialog />
{/if}
<SourcePicker />
<WindowsFileDialog />
<PrintDialog />
<MenuPalette />
<VoiceHud />
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
