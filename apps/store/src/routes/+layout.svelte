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
  import { initArlenTheme } from "@arlen/ui-kit/theme";

  let { children } = $props();

  // The chosen language and the live theme, the same two lines every other
  // app runs. The German catalogue in messages.ts was unreachable without
  // the first one.
  onMount(() => {
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
