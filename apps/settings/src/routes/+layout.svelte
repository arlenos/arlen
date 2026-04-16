<script lang="ts">
  import "../app.css";
  import { page } from "$app/stores";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import AppSidebar from "$lib/components/AppSidebar.svelte";
  import SiteHeader from "$lib/components/SiteHeader.svelte";
  import {
    SidebarProvider,
    SidebarInset,
  } from "$lib/components/ui/sidebar";
  import {
    syncFromRoute,
    breadcrumbs,
    navigateTo,
    type PanelId,
  } from "$lib/stores/navigation";
  import { theme } from "$lib/stores/theme";
  import { exportSettingsIndex } from "$lib/search/index";

  let { children } = $props();

  // Sync navigation store with route changes.
  $effect(() => {
    syncFromRoute($page.url.pathname);
  });

  // Push breadcrumb updates to the Lunaris titlebar plugin. Under the
  // Lunaris compositor this renders segments in the global top bar.
  // Under other compositors the plugin is a no-op and we fall back to
  // the in-app breadcrumb in SiteHeader.
  $effect(() => {
    const segments = $breadcrumbs.map((label) => ({ label }));
    invoke("plugin:lunaris-menu|set_breadcrumb", {
      segmentsJson: JSON.stringify(segments),
    }).catch(() => {});
  });

  onMount(() => {
    theme.load();

    // Export the settings search index so Waypointer always has an
    // up-to-date copy at ~/.local/share/lunaris/settings-index.json.
    exportSettingsIndex();

    // Live reload on config watcher events from the backend.
    let unlistenAppearance: UnlistenFn | undefined;

    listen("config:appearance:changed", () => {
      theme.load();
    }).then((fn) => {
      unlistenAppearance = fn;
    });

    // Deep link navigation from CLI args. The backend stashes them
    // in a static and we pull them here — guaranteed to run after
    // mount, so the DOM is ready and no race is possible.
    invoke<{ panel: string; anchor: string | null } | null>(
      "get_launch_args",
    ).then((target) => {
      if (!target) return;
      console.log("[deep-link] launch args:", target.panel, target.anchor);
      navigateTo(target.panel as PanelId).then(() => {
        if (!target.anchor) return;
        pollForElement(target.anchor, 2000).then((el) => {
          if (el) {
            scrollToSetting(el);
          } else {
            console.warn(
              `[deep-link] element #${target.anchor} not found after 2s`,
            );
          }
        });
      });
    });

    return () => {
      unlistenAppearance?.();
    };
  });

  /// Poll for a DOM element by ID. SvelteKit renders the new page
  /// asynchronously after `goto()` resolves, so we cannot assume the
  /// element exists immediately. Returns `null` if not found within
  /// the timeout.
  function pollForElement(
    id: string,
    timeoutMs: number,
  ): Promise<HTMLElement | null> {
    return new Promise((resolve) => {
      const start = performance.now();
      function check() {
        const el = document.getElementById(id);
        if (el) {
          resolve(el);
        } else if (performance.now() - start > timeoutMs) {
          resolve(null);
        } else {
          requestAnimationFrame(check);
        }
      }
      check();
    });
  }

  /// Scroll to the given element within the content scroll container
  /// and briefly highlight it with a pulse animation.
  function scrollToSetting(el: HTMLElement) {
    // The actual scroll container is the flex-1 overflow-y-auto div,
    // not the document body (which has overflow:hidden). Find the
    // nearest scrollable ancestor.
    const container = el.closest(".overflow-y-auto") ?? el.parentElement;
    if (container) {
      const top =
        el.getBoundingClientRect().top -
        container.getBoundingClientRect().top +
        container.scrollTop -
        container.clientHeight / 3;
      container.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
    }

    // Force reflow before adding the class so the browser starts
    // the animation from the current computed state, not from a
    // cached/optimised no-op.
    void el.offsetHeight;
    el.classList.add("setting-highlight");
    console.log(
      "[deep-link] highlighted #" + el.id,
      "classes:", el.classList.toString(),
      "bg:", getComputedStyle(el).backgroundColor,
    );
    setTimeout(() => el.classList.remove("setting-highlight"), 2500);
  }
</script>

<SidebarProvider class="h-screen min-h-0 overflow-hidden">
  <AppSidebar />
  <SidebarInset class="h-screen min-h-0 overflow-hidden">
    <SiteHeader />
    <div class="min-h-0 flex-1 overflow-y-auto">
      {@render children()}
    </div>
  </SidebarInset>
</SidebarProvider>
