<script lang="ts">
  /// The console sidebar: SESSIONS (the running shells, sessions are
  /// the tabs), HISTORY (search over past blocks, Ctrl+R) and
  /// PROJECTS (scopes the history). Sessions carry their cwd short
  /// form, a status dot and the last exit code; history rows hand
  /// their command to the composer on click.
  import { onMount, tick } from "svelte";
  import { writable } from "svelte/store";
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
    useSidebar,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Toggle } from "@arlen/ui-kit/components/ui/toggle";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { Folder, Plus, TerminalSquare } from "lucide-svelte";
  import { terminalProjects, type Project } from "$lib/contract";
  import {
    sessions,
    activeSessionId,
    sessionsLoaded,
    newSession,
    selectSession,
  } from "$lib/stores/sessions";
  import {
    historyQuery,
    historyOnlyFailures,
    historyAgentOnly,
    historyProjectId,
    historyResults,
    historyLoaded,
    historyFocusTick,
    runHistorySearch,
    queueHistorySearch,
  } from "$lib/stores/history";
  import { prefillComposer } from "$lib/stores/composer";

  const sidebar = useSidebar();
  let searchRef = $state<HTMLInputElement | null>(null);

  const projects = writable<Project[]>([]);
  const projectsLoaded = writable(false);

  /// How many history rows the sidebar shows at once; the count line
  /// below the list names what stays hidden.
  const HISTORY_LIMIT = 8;

  onMount(async () => {
    runHistorySearch();
    try {
      projects.set(await terminalProjects());
    } catch {
      // Unreachable backend: the group keeps its honest empty line.
    }
    projectsLoaded.set(true);
  });

  // Ctrl+R (the layout bumps the tick): open the sidebar if it is
  // collapsed, then focus the search field.
  $effect(() => {
    if ($historyFocusTick > 0) {
      sidebar.open = true;
      tick().then(() => searchRef?.focus());
    }
  });

  function toggleProject(id: string) {
    historyProjectId.update((cur) => (cur === id ? null : id));
    queueHistorySearch();
  }

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

    <SidebarGroup class="group-data-[collapsible=icon]:hidden">
      <SidebarGroupLabel>History</SidebarGroupLabel>
      <div class="ts-search">
        <Input
          id="terminal-history-search"
          bind:ref={searchRef}
          bind:value={$historyQuery}
          class="h-7 text-xs"
          placeholder="Search history"
          aria-label="Search history (Ctrl+R)"
          oninput={() => queueHistorySearch()}
        />
        <div class="ts-chips">
          <Toggle
            id="terminal-history-failures"
            bind:pressed={$historyOnlyFailures}
            class="ts-chip"
            aria-label="Only failed commands"
            onPressedChange={() => queueHistorySearch()}
          >
            Failures
          </Toggle>
          <Toggle
            id="terminal-history-agent"
            bind:pressed={$historyAgentOnly}
            class="ts-chip"
            aria-label="Only commands the agent ran"
            onPressedChange={() => queueHistorySearch()}
          >
            Agent
          </Toggle>
        </div>
      </div>
      <SidebarMenu>
        {#if $historyLoaded && $historyResults.length === 0}
          <div class="ts-empty">No matching commands.</div>
        {/if}
        {#each $historyResults.slice(0, HISTORY_LIMIT) as b (b.id)}
          <SidebarMenuItem>
            <SidebarMenuButton
              tooltip={b.command}
              onclick={() => prefillComposer(b.command)}
            >
              <span class="ts-session-label">
                <span class="ts-session-cwd">
                  {#if b.origin === "agent"}<span class="ts-agent" aria-hidden="true">✦</span>{/if}
                  {b.command}
                </span>
                <span class="ts-session-meta">
                  {shortCwd(b.cwd)}
                  {#if b.exit_code !== null && b.exit_code !== 0}
                    <span class="ts-exit">exit {b.exit_code}</span>
                  {/if}
                </span>
              </span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
        {#if $historyResults.length > HISTORY_LIMIT}
          <div class="ts-more">
            Showing {HISTORY_LIMIT} of {$historyResults.length}.
          </div>
        {/if}
      </SidebarMenu>
    </SidebarGroup>

    <SidebarGroup class="group-data-[collapsible=icon]:hidden">
      <SidebarGroupLabel>Projects</SidebarGroupLabel>
      <SidebarMenu>
        {#if $projectsLoaded && $projects.length === 0}
          <div class="ts-empty">No projects yet.</div>
        {/if}
        {#each $projects as p (p.id)}
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={p.id === $historyProjectId}
              tooltip={p.path}
              onclick={() => toggleProject(p.id)}
            >
              <Folder />
              <span class="ts-session-label">
                <span class="ts-project-name">{p.name}</span>
                <span class="ts-session-meta">{shortCwd(p.path)}</span>
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

  .ts-search {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 8px 6px;
  }
  .ts-chips {
    display: flex;
    gap: 6px;
  }
  .ts-search :global(.ts-chip) {
    height: var(--height-control-compact, 24px);
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-chip);
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--sidebar-foreground) 60%, transparent);
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
  .ts-agent {
    color: var(--color-accent, var(--primary));
  }
  .ts-exit {
    color: var(--color-error);
  }
  .ts-more {
    padding: 4px 8px 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--sidebar-foreground) 45%, transparent);
  }
  .ts-project-name {
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ts-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-success);
    flex-shrink: 0;
  }
  .ts-dot-exited {
    background: color-mix(in srgb, var(--sidebar-foreground) 30%, transparent);
  }
  .ts-dot-failed {
    background: var(--color-error);
  }
</style>
