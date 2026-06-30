<script lang="ts">
  /// The console sidebar: the session switcher (terminal.md §4.3,
  /// sessions are the tabs) and nothing else — history lives in the
  /// Ctrl+R palette, reachable from the footer row. One-line rows on
  /// one text edge; the right-hand dot rail carries session status
  /// (green running, gray exited, red failed) and the tooltip carries
  /// what the dot alone cannot say.
  import {
    Sidebar,
    SidebarContent,
    SidebarFooter,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { Plus } from "lucide-svelte";
  import type { Session } from "$lib/contract";
  import { displayPath } from "$lib/paths";
  import {
    sessions,
    activeSessionId,
    sessionsLoaded,
    newSession,
    selectSession,
  } from "$lib/stores/sessions";
  import { openHistoryPalette } from "$lib/stores/history";

  function sessionFailed(s: Session): boolean {
    return s.status === "running" && s.last_exit !== null && s.last_exit !== 0;
  }

  /// The tooltip says what the dot cannot: which state, and which
  /// exit code when the last command failed.
  function sessionTitle(s: Session): string {
    if (s.status === "exited") return `${s.cwd} (exited)`;
    if (sessionFailed(s)) return `${s.cwd} (last command exited ${s.last_exit})`;
    return s.cwd;
  }
</script>

<!-- Sessions are tabs (terminal.md §4.3). With a single session the rail is
     pure chrome, so collapsed it slides fully off-canvas (no visible sidebar);
     expanded it still carries the one tab. With two or more it collapses to the
     icon dot-rail instead, so you can switch sessions without expanding. -->
<Sidebar collapsible={$sessions.length > 1 ? "icon" : "offcanvas"}>
  <SidebarHeader class="h-10 flex-row items-center justify-between py-0">
    <span
      class="px-2 text-[0.6875rem] font-semibold uppercase tracking-[0.1em] text-sidebar-foreground/55 group-data-[collapsible=icon]:hidden"
    >
      Terminal
    </span>
    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <button
            {...props}
            id="terminal-new-session"
            class="ts-new-btn"
            aria-label="New session"
            onclick={() => newSession()}
          >
            <Plus size={14} strokeWidth={2} />
          </button>
        {/snippet}
      </Tooltip.Trigger>
      <Tooltip.TooltipContent side="bottom">
        New session (Ctrl+T)
      </Tooltip.TooltipContent>
    </Tooltip.Root>
  </SidebarHeader>

  <SidebarContent>
    <SidebarGroup>
      <SidebarGroupLabel>Sessions</SidebarGroupLabel>
      <SidebarMenu>
        {#if $sessionsLoaded && $sessions.length === 0}
          <div class="ts-empty group-data-[collapsible=icon]:hidden">
            No open sessions. Start one with the plus button or Ctrl+T.
          </div>
        {/if}
        {#each $sessions as s (s.id)}
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={s.id === $activeSessionId}
              tooltip={sessionTitle(s)}
              onclick={() => selectSession(s.id)}
            >
              <!-- Collapsed rail: the dot IS the session. Expanded: it
                   sits on the right rail, clear of the truncating text. -->
              <span
                class="ts-dot hidden group-data-[collapsible=icon]:mx-auto group-data-[collapsible=icon]:block"
                class:ts-dot-exited={s.status === "exited"}
                class:ts-dot-failed={sessionFailed(s)}
              ></span>
              <span class="ts-text group-data-[collapsible=icon]:hidden">
                {displayPath(s.cwd)}
              </span>
              <span
                class="ts-dot ml-auto group-data-[collapsible=icon]:hidden"
                class:ts-dot-exited={s.status === "exited"}
                class:ts-dot-failed={sessionFailed(s)}
              ></span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
    </SidebarGroup>
  </SidebarContent>

  <SidebarFooter class="group-data-[collapsible=icon]:hidden">
    <button
      id="terminal-history-open"
      class="ts-footer-row"
      onclick={() => openHistoryPalette()}
    >
      <span>History</span>
      <span class="ts-footer-hint">Ctrl+R</span>
    </button>
  </SidebarFooter>

  <SidebarRail />
</Sidebar>

<style>
  /* The nav register (the same square the Settings sidebar gives its
     collapsed search): 32px box, 14px glyph. */
  .ts-new-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--sidebar-foreground) 55%, transparent);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-new-btn:hover {
    background: color-mix(in srgb, var(--sidebar-foreground) 10%, transparent);
    color: var(--sidebar-foreground);
  }

  /* Row text: console content voice (13px mono), truncating. The kit
     button's text-sm never reaches a bare text node. */
  .ts-text {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ts-empty {
    padding: 4px 8px;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--sidebar-foreground) 55%, transparent);
  }

  /* The one dot language: green alive, gray exited, red failed.
     Fills are deliberately outside the text dim scale. */
  .ts-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-success);
    flex-shrink: 0;
  }
  .ts-dot-exited {
    background: color-mix(in srgb, var(--sidebar-foreground) 40%, transparent);
  }
  .ts-dot-failed {
    background: var(--color-error);
  }

  .ts-footer-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: var(--height-control, 28px);
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--sidebar-foreground) 55%, transparent);
    font-size: 0.75rem;
    font-weight: 500;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-footer-row:hover {
    background: color-mix(in srgb, var(--sidebar-foreground) 8%, transparent);
    color: var(--sidebar-foreground);
  }
  .ts-footer-hint {
    font-weight: 400;
    color: color-mix(in srgb, var(--sidebar-foreground) 35%, transparent);
  }
</style>
