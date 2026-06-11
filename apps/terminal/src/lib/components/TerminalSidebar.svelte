<script lang="ts">
  /// The console sidebar: SESSIONS (the running shells, sessions are
  /// the tabs), HISTORY (search over past blocks) and PROJECTS
  /// (graph-backed scopes). Sessions carry their cwd short form, a
  /// status dot and the last exit code.
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { Plus, TerminalSquare } from "lucide-svelte";
  import {
    sessions,
    activeSessionId,
    sessionsLoaded,
    newSession,
    selectSession,
  } from "$lib/stores/sessions";

  /// Last two path segments, like the recent-files rows elsewhere.
  function shortCwd(p: string): string {
    const parts = p.split("/").filter((x) => x.length > 0);
    if (parts.length === 0) return "/";
    if (parts.length === 1) return "/" + parts[0];
    return parts[parts.length - 2] + "/" + parts[parts.length - 1];
  }
</script>

<Sidebar collapsible="icon">
  <SidebarHeader class="h-10 flex-row items-center justify-between py-0">
    <span
      class="px-2 text-xs font-semibold tracking-wide text-sidebar-foreground/70 group-data-[collapsible=icon]:hidden"
    >
      Terminal
    </span>
    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <button
            {...props}
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
            No open shells. Start one with the plus button or Ctrl+T.
          </div>
        {/if}
        {#each $sessions as s (s.id)}
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={s.id === $activeSessionId}
              tooltip={s.cwd}
              onclick={() => selectSession(s.id)}
            >
              <TerminalSquare />
              <span class="ts-session-label">
                <span class="ts-session-cwd">{shortCwd(s.cwd)}</span>
                <span class="ts-session-meta">
                  <span
                    class="ts-dot"
                    class:ts-dot-exited={s.status === "exited"}
                    class:ts-dot-failed={s.status === "running" &&
                      s.last_exit !== null &&
                      s.last_exit !== 0}
                  ></span>
                  {#if s.status === "exited"}
                    exited
                  {:else if s.last_exit !== null && s.last_exit !== 0}
                    exit {s.last_exit}
                  {:else}
                    running
                  {/if}
                </span>
              </span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
    </SidebarGroup>
  </SidebarContent>

  <SidebarRail />
</Sidebar>

<style>
  .ts-new-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--sidebar-foreground) 60%, transparent);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-new-btn:hover {
    background: color-mix(in srgb, var(--sidebar-foreground) 10%, transparent);
    color: var(--sidebar-foreground);
  }

  .ts-empty {
    padding: 6px 8px;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--sidebar-foreground) 50%, transparent);
  }

  .ts-session-label {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .ts-session-cwd {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ts-session-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--sidebar-foreground) 50%, transparent);
  }
  .ts-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-success, #10b981);
    flex-shrink: 0;
  }
  .ts-dot-exited {
    background: color-mix(in srgb, var(--sidebar-foreground) 30%, transparent);
  }
  .ts-dot-failed {
    background: var(--color-error, #ef4444);
  }
</style>
