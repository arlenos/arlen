<script lang="ts">
  /// The console sidebar: SESSIONS (the running shells, sessions are
  /// the tabs), HISTORY (search over past blocks, Ctrl+R) and
  /// PROJECTS (scopes the history). One-line rows on one text edge;
  /// the right-hand dot rail carries session status (green running,
  /// gray exited, red failed) and the tooltip carries what the dot
  /// alone cannot say.
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
  import { Plus } from "lucide-svelte";
  import { terminalProjects, type Project, type Session } from "$lib/contract";
  import { shortPath } from "$lib/paths";
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

  /// Whether the user narrowed the history; decides between the
  /// no-matches line and the nothing-here-yet line.
  const historyFiltered = $derived(
    $historyQuery.trim().length > 0 ||
      $historyOnlyFailures ||
      $historyAgentOnly ||
      $historyProjectId !== null,
  );

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

<Sidebar collapsible="icon">
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
                {shortPath(s.cwd)}
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
          <div class="ts-empty">
            {historyFiltered ? "No matching commands." : "Nothing to search yet."}
          </div>
        {/if}
        {#each $historyResults.slice(0, HISTORY_LIMIT) as b (b.id)}
          <SidebarMenuItem>
            <SidebarMenuButton
              tooltip={`${b.command} (in ${shortPath(b.cwd)})`}
              onclick={() => prefillComposer(b.command)}
            >
              <span class="ts-text">{b.command}</span>
              {#if b.exit_code !== null && b.exit_code !== 0}
                <span class="ts-exit ml-auto">exit {b.exit_code}</span>
              {/if}
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
        {#if $historyResults.length > HISTORY_LIMIT}
          <div class="ts-more">
            Showing {HISTORY_LIMIT} of {$historyResults.length}. Search to see
            the rest.
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
              <span class="ts-text">{p.name}</span>
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

  .ts-search {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0 0 8px;
  }
  /* Text edge law: group 8px + border 1px + 7px = 16px, the same
     edge every other sidebar text sits on. */
  .ts-search :global(input) {
    padding-inline: 7px;
  }
  .ts-chips {
    display: flex;
    gap: 8px;
  }
  .ts-search :global(.ts-chip) {
    height: var(--height-control-compact, 24px);
    padding: 0 7px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-chip);
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--sidebar-foreground) 55%, transparent);
  }
  .ts-search :global(.ts-chip[data-state="on"]) {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent, var(--primary)) 35%, transparent);
    color: var(--color-accent, var(--primary));
  }

  .ts-exit {
    flex-shrink: 0;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: var(--color-error);
  }
  .ts-more {
    padding: 4px 8px 0;
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
    background: color-mix(in srgb, var(--sidebar-foreground) 30%, transparent);
  }
  .ts-dot-failed {
    background: var(--color-error);
  }
</style>
