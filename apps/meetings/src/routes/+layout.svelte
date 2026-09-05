<script lang="ts">
  /// App shell on the sidebar grammar (the harness/settings skeleton): the
  /// meeting list lives in the rail with Start a meeting pinned on top, the
  /// inset header names the open surface once and carries the window controls.
  /// The window runs with `decorations: false`, so the header is also the drag
  /// region - an explicit `startDragging()` pointerdown rather than the
  /// `data-tauri-drag-region` attribute, guarded so vite still renders.
  ///
  /// The attribute is not so much unreliable as NARROW. Tauri's injected handler
  /// (tauri 2.10.3 `window/scripts/drag.js`) delegates on `document` and tests
  /// `e.target.getAttribute(...)`, the exact element under the pointer, walking no
  /// ancestors - so a press on any child, a label, an icon, the gap inside a nested
  /// flex box, is not a press on the drag region. A header built from nested boxes
  /// drags only on the slivers of itself that show through. The handler here uses
  /// `closest`, which DOES walk, so it drags from anywhere except the controls.
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
  import { appMenuGroups, registerAppMenu, initMenuActions, menuAction } from "$lib/menu";
  import { setWindowTitle } from "$lib/window-title";
  import {
    captureUnavailable,
    transcribe,
    stopCapture,
    openInEditor,
    meetings,
    meetingsMocked,
    meetingsUnavailable,
    meetingsFailure,
    meetingsFailureKey,
    loadMeetings,
    meeting,
    fmtDate,
  } from "$lib/stores/meeting";
  import { locale } from "$lib/i18n/messages";

  let { children } = $props();

  // The topbar and the workspace overview show the NATIVE window title,
  // not the document one below, so it has to be set - and set again when
  // the language changes, which is why this reads `$t` instead of firing
  // once at startup.
  $effect(() => {
    void setWindowTitle($t("mt.app.title"));
  });

  // The shell menu, re-registered whenever the language OR the transcribe
  // state changes: the checked mark is part of the registered tree.
  $effect(() => {
    void registerAppMenu(appMenuGroups($t, { transcribe: $transcribe }));
  });
  // Its dispatch: the same verbs the rail and the capture surface run.
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "meeting.start") void goto("/capture");
    else if (a === "meeting.stop") void stopCapture();
    else if (a === "meeting.open_editor") void openInEditor();
    else if (a === "view.transcribe") transcribe.update((v) => !v);
  });

  onMount(() => {
    void initMenuActions();
    void loadMeetings();
    // The chosen language and the live theme, the same two lines every other app
    // runs. This app embeds the plugin and has the permission; it just never
    // asked, so it stayed English on a German desktop.
    void initArlenLocale();
    void initArlenTheme();
  });

  const path = $derived($page.url.pathname);
  const activeId = $derived(path.startsWith("/meeting/") ? path.slice("/meeting/".length) : null);
  // Being ON the capture route is not the same as capturing. The pill carries a
  // red dot and the word "Recording", which is a claim about a microphone, and
  // the URL knows nothing about one - so a refused capture showed the page's own
  // "Recording did not start" beside a sidebar still saying Recording.
  const capturing = $derived(path === "/capture" && !$captureUnavailable);
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

<svelte:head>
  <!-- The document title: what a screen reader announces for the window
       and what a task switcher shows. Every Arlen app was missing one,
       which axe reports as `document-title` on every surface. -->
  <title>{$t("mt.app.title")}</title>
</svelte:head>

<div dir={$dir} style="display: contents">
  <SidebarProvider class="h-screen min-h-0 overflow-hidden">
    <!-- Offcanvas, not the icon rail: meeting rows are titles with no leading
         icon, so a 3rem rail would render wrapped text fragments instead of a
         usable strip. Same reasoning as the harness chat history. -->
    <Sidebar>
      <SidebarContent>
        <!-- The first group keeps a 6px top inset so its row clears the
         window edge by the same gap as the header-bar icons; the row itself
         stays the rail's uniform 32px box - an earlier pass shrank only the
         first box to 28px and it read as broken, smaller and shifted against
         its siblings. -->
        <SidebarGroup class="pt-1.5">
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
          {#if $meetingsMocked}
            <!-- Beside the list, not in the empty pane on the other side of the
                 window: the rows are here, they are named and dated and
                 clickable, and a reader scanning them has no reason to look at
                 the pane that says "pick one". -->
            <p class="mt-sample">{$t("mt.sample.list")}</p>
          {:else if $meetingsUnavailable}
            <!-- Same place, for the same reason, and it is a different sentence:
                 "these are examples" and "I could not read yours" are different
                 facts and only one of them is about this machine. -->
            <!-- And it names its cause when it has one. The catch that sets this
                 flag used to drop the error on the floor, so the sentence could
                 not tell a daemon that is down from a permission you do not have
                 from a store that is corrupt - three different things to do about
                 it, rendered as one shrug. -->
            <p class="mt-sample">
              {$t(meetingsFailureKey($meetingsFailure))}
            </p>
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
          {#if $meetings.length === 0 && !$meetingsUnavailable}
            <!-- Not when the read failed: "No meetings yet" is a claim about your
                 history, and the line above has just said that history could not
                 be read. Three states kept apart - not loaded, loaded and failed,
                 loaded and empty - and this is the third. -->
            <!-- The SHORT form. The main pane already says the whole sentence and
                 carries the button that acts on it; both were saying the same
                 thing at once, which reads as two different pieces of news until
                 you have read both. The list says what the list holds. -->
            <p class="px-2 py-2 text-xs leading-relaxed text-sidebar-foreground/55">{$t("mt.emptyShort")}</p>
          {/if}
        </SidebarGroup>
      </SidebarContent>
      <SidebarRail />
    </Sidebar>

    <SidebarInset class="h-svh min-h-0">
      <!-- The page's one level-one heading. Every app in this tree had none, so a
           screen reader's first question - what IS this window - was answered only
           by the window title, which is not in the document. The app NAME rather
           than the visible bar title, which says where you are inside the app and
           changes as you move; hidden, because the bar already shows that and a
           second visible title would be the same fact twice. -->
      <h1 class="sr-only">{$t("mt.app.title")}</h1>
      <!-- The header is a drag surface (a non-keyboard pointer interaction); its
           actual controls are the accessible WindowButtons, so the
           static-interaction lint is a false positive here. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <header
        onpointerdown={startDrag}
        ondblclick={toggleMax}
        class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background px-2"
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
