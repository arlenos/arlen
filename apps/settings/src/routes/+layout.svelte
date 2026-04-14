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
  import { syncFromRoute, breadcrumbs } from "$lib/stores/navigation";
  import { theme } from "$lib/stores/theme";

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

    // Live reload on config watcher events from the backend.
    let unlistenAppearance: UnlistenFn | undefined;
    listen("config:appearance:changed", () => {
      theme.load();
    }).then((fn) => {
      unlistenAppearance = fn;
    });

    return () => {
      unlistenAppearance?.();
    };
  });
</script>

<SidebarProvider>
  <AppSidebar />
  <SidebarInset>
    <SiteHeader />
    <div class="flex-1 overflow-y-auto">
      {@render children()}
    </div>
  </SidebarInset>
</SidebarProvider>
