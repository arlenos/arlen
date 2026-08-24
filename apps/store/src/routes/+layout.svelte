<script lang="ts">
  /// App shell on the files canon, hoisted here so EVERY route carries the
  /// same chrome (the app detail page used to render without the rail at
  /// all): provider, the places sidebar, the h-10 header naming the place.
  import "../app.css";
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { SidebarProvider, SidebarInset } from "@arlen/ui-kit/components/ui/sidebar";
  import StoreRail from "$lib/components/StoreRail.svelte";
  import StoreHeader from "$lib/components/StoreHeader.svelte";
  import { apps } from "$lib/stores/catalog";
  import { t, dir } from "$lib/i18n/messages";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { goto } from "$app/navigation";
  import { initAppMenu, menuAction } from "$lib/menu";
  import { loadCatalog } from "$lib/stores/catalog";
  import { loadUpdates, applyAllRoutine } from "$lib/stores/updates";
  import { initArlenTheme } from "@arlen/ui-kit/theme";

  let { children } = $props();

  // The chosen language and the live theme, the same two lines every other
  // app runs. The German catalogue in messages.ts was unreachable without
  // the first one.
  // The shell menu's dispatch: refresh and update verbs work from any route.
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "store.refresh") void loadCatalog();
    else if (a === "store.check") void loadUpdates();
    else if (a === "store.update_all") void applyAllRoutine();
    else if (a === "go.browse") void goto("/");
    else if (a === "go.installed") void goto("/installed");
    else if (a === "go.updates") void goto("/updates");
  });

  onMount(() => {
    void initAppMenu();
    void initArlenLocale();
    void initArlenTheme();
  });

  // The bar names the place; on an app page, the app.
  const placeLabel = $derived.by(() => {
    const path = $page.url.pathname;
    if (path.startsWith("/app/")) {
      const id = path.slice("/app/".length);
      return $apps.find((a) => a.id === id)?.name ?? $t("st.rail.browse");
    }
    if (path === "/installed") return $t("st.rail.installed");
    if (path === "/updates") return $t("st.rail.updates");
    return $t("st.rail.browse");
  });
</script>

<div dir={$dir} style="display: contents">
  <SidebarProvider class="h-screen min-h-0 overflow-hidden">
    <StoreRail />
    <SidebarInset class="h-svh min-h-0">
      <StoreHeader {placeLabel} />
      {@render children()}
    </SidebarInset>
  </SidebarProvider>
</div>
