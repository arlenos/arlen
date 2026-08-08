<script lang="ts">
  /// App shell on the sidebar grammar (the harness/settings skeleton): the
  /// meeting list lives in the rail with Start a meeting pinned on top, the
  /// inset header names the open surface once and carries the window controls.
  /// The window runs with `decorations: false`, so the header is also the drag
  /// region - an explicit `startDragging()` pointerdown (the
  /// `data-tauri-drag-region` attribute is unreliable on Wayland in Tauri v2),
  /// guarded so vite still renders.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarInset,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarProvider,
    SidebarRail,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Plus } from "lucide-svelte";
  import { t, dir } from "$lib/i18n/messages";
  import { meetings, meetingsMocked, loadMeetings, meeting, fmtDate } from "$lib/stores/meeting";
  import { locale } from "$lib/i18n/messages";

  let { children } = $props();

  onMount(() => {
    void loadMeetings();
    // The chosen language and the live theme, the same two lines every other app
    // runs. This app embeds the plugin and has the permission; it just never
    // asked, so it stayed English on a German desktop.
    void initArlenLocale();
    void initArlenTheme();
  });

  const path = $derived($page.url.pathname);
  const activeId = $derived(path.startsWith("/meeting/") ? path.slice("/meeting/".length) : null);
  const capturing = $derived(path === "/capture");
  // The surface title, said once: the recording surface, the open note or the app.
  const title = $derived(
    capturing ? $t("mt.newMeeting") : activeId && $meeting ? $meeting.note.title : $t("mt.title")
  );

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }

  async function startDrag(e: PointerEvent) {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* standalone (vite) has no toplevel to drag */
    }
  }

  async function toggleMax(e: MouseEvent) {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      /* no window in standalone */
    }
  }
</script>

<div dir={$dir} style="display: contents">
  <SidebarProvider class="h-screen min-h-0 overflow-hidden">
    <Sidebar>
      <SidebarContent>
        <SidebarGroup>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton id="start-meeting" isActive={capturing} onclick={() => goto("/capture")}>
                {#if capturing}
                  <span class="rec-dot" aria-hidden="true"></span>
                  <span>{$t("mt.recording")}</span>
                {:else}
                  <Plus strokeWidth={2} />
                  <span>{$t("mt.start")}</span>
                {/if}
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>

        <SidebarGroup class="pt-0">
          <SidebarGroupLabel>{$t("mt.title")}</SidebarGroupLabel>
          {#if $meetingsMocked}
            <!-- Beside the list, not in the empty pane on the other side of the
                 window: the rows are here, they are named and dated and
                 clickable, and a reader scanning them has no reason to look at
                 the pane that says "pick one". -->
            <p class="mt-sample">{$t("mt.sample.list")}</p>
          {/if}
          <SidebarMenu>
            {#each $meetings as m (m.id)}
              <SidebarMenuItem>
                <SidebarMenuButton
                  id={`meeting-${m.id}`}
                  isActive={activeId === m.id}
                  onclick={() => goto(`/meeting/${m.id}`)}
                >
                  <span class="truncate">{m.title}</span>
                  <span class="ms-auto shrink-0 text-xs text-sidebar-foreground/50">{fmtDate(m.date_ms, $locale)}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            {/each}
          </SidebarMenu>
          {#if $meetings.length === 0}
            <p class="px-2 py-2 text-xs leading-relaxed text-sidebar-foreground/55">{$t("mt.empty")}</p>
          {/if}
        </SidebarGroup>
      </SidebarContent>
      <SidebarRail />
    </Sidebar>

    <SidebarInset class="h-svh min-h-0">
      <!-- The header is a drag surface (a non-keyboard pointer interaction); its
           actual controls are the accessible WindowButtons, so the
           static-interaction lint is a false positive here. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <header
        onpointerdown={startDrag}
        ondblclick={toggleMax}
        class="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-background px-2"
      >
        <SidebarTrigger class="-ml-1" />
        <Separator orientation="vertical" class="me-1 h-4" />
        <span class="select-none truncate text-sm font-medium text-foreground">{title}</span>
        <div class="flex-1"></div>
        <WindowButtons />
      </header>
      <div class="min-h-0 flex-1 overflow-y-auto">
        {@render children()}
      </div>
    </SidebarInset>
  </SidebarProvider>
</div>

<style>
  .mt-sample {
    margin: 0 8px 4px;
    font-size: 11px;
    line-height: 1.35;
    color: color-mix(in srgb, currentColor 55%, transparent);
  }

  .rec-dot {
    width: 0.5rem;
    height: 0.5rem;
    flex-shrink: 0;
    border-radius: var(--radius-full, 9999px);
    background: var(--color-error, #dc2626);
    animation: rec-pulse 1.6s ease-in-out infinite;
  }
  @keyframes rec-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rec-dot {
      animation: none;
    }
  }
</style>
