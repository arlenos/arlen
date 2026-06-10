<script lang="ts">
  /// The harness sidebar (ai-app.md §2.0): surface nav (Chat / Agent) above
  /// the conversation history, on the `@arlen/ui-kit` sidebar canon. The
  /// history is the chat archetype's contextual column folded in: new chat,
  /// a title/content search, and the pinned-first session list with per-row
  /// pin / rename / delete behind a quiet hover menu.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuAction,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import { MessageSquare, MoreHorizontal, Pencil, Pin, PinOff, Plus, Trash2 } from "@lucide/svelte";
  import {
    orderedSessions,
    activeSessionId,
    newSession,
    selectSession,
    deleteSession,
    renameSession,
    togglePinSession,
  } from "$lib/stores/conversation";
  import { sessionMatches } from "$lib/search";

  const SURFACES = [
    { value: "/", label: "Chat" },
    { value: "/agent", label: "Agent" },
  ];
  const surface = $derived($page.url.pathname.startsWith("/agent") ? "/agent" : "/");

  let query = $state("");
  // The conversation being renamed inline, and the draft title. `null` when no
  // rename is in progress; double-clicking a title or the row menu opens it.
  let editingId = $state<string | null>(null);
  let draft = $state("");

  function beginRename(id: string, current: string): void {
    editingId = id;
    draft = current;
  }
  function commitRename(): void {
    if (editingId !== null) renameSession(editingId, draft);
    editingId = null;
  }
  function cancelRename(): void {
    editingId = null;
  }

  // Selecting a session (or starting a new one) belongs to the Chat surface,
  // so either navigates there when the Agent surface is active.
  function openSession(id: string): void {
    selectSession(id);
    if (surface !== "/") goto("/");
  }
  function startNew(): void {
    newSession();
    if (surface !== "/") goto("/");
  }

  // Sessions in rail order (pinned first), narrowed by the search. The query
  // matches titles and message content, case-insensitive; empty matches all.
  const filtered = $derived($orderedSessions.filter((s) => sessionMatches(s, query)));
</script>

<Sidebar>
  <SidebarHeader>
    <SegmentedControl
      id="harness-surface-nav"
      class="w-full *:flex-1"
      options={SURFACES}
      value={surface}
      ariaLabel="Surface"
      onchange={(v) => goto(v)}
    />
    <Button
      id="harness-new-chat"
      variant="outline"
      size="sm"
      class="w-full justify-start gap-1.5"
      title="New chat (Ctrl+N)"
      onclick={startNew}
    >
      <Plus size={14} strokeWidth={2} />
      New chat
    </Button>
    {#if $orderedSessions.length > 0}
      <Input
        id="harness-session-search"
        class="h-8"
        bind:value={query}
        placeholder="Search conversations"
        aria-label="Search conversations"
      />
    {/if}
  </SidebarHeader>

  <SidebarContent>
    <SidebarGroup>
      <SidebarGroupLabel>History</SidebarGroupLabel>
      {#if $orderedSessions.length === 0}
        <p class="px-2 py-3 text-xs leading-relaxed text-sidebar-foreground/60">
          No conversations yet. Ask something to start one.
        </p>
      {:else if filtered.length === 0}
        <p class="px-2 py-3 text-xs text-sidebar-foreground/60">No conversations match.</p>
      {:else}
        <SidebarMenu>
          {#each filtered as s (s.id)}
            <SidebarMenuItem>
              {#if editingId === s.id}
                <Input
                  class="h-8"
                  bind:value={draft}
                  aria-label="Rename conversation"
                  onblur={commitRename}
                  onkeydown={(e: KeyboardEvent) => {
                    if (e.key === "Enter") commitRename();
                    else if (e.key === "Escape") cancelRename();
                  }}
                  {@attach (node: HTMLInputElement) => {
                    node.focus();
                    node.select();
                  }}
                />
              {:else}
                <SidebarMenuButton
                  class="pr-7"
                  isActive={s.id === $activeSessionId}
                  title={s.title}
                  onclick={() => openSession(s.id)}
                  ondblclick={() => beginRename(s.id, s.title)}
                >
                  <MessageSquare strokeWidth={1.75} />
                  <span class="truncate">{s.title}</span>
                  {#if s.pinned}
                    <Pin strokeWidth={1.75} class="ml-auto opacity-50" aria-label="Pinned" />
                  {/if}
                </SidebarMenuButton>
                <DropdownMenu.Root>
                  <DropdownMenu.Trigger>
                    {#snippet child({ props })}
                      <SidebarMenuAction showOnHover aria-label="Conversation actions" {...props}>
                        <MoreHorizontal strokeWidth={2} />
                      </SidebarMenuAction>
                    {/snippet}
                  </DropdownMenu.Trigger>
                  <DropdownMenu.Content side="right" align="start">
                    <DropdownMenu.Item onclick={() => togglePinSession(s.id)}>
                      {#if s.pinned}
                        <PinOff />
                        Unpin
                      {:else}
                        <Pin />
                        Pin
                      {/if}
                    </DropdownMenu.Item>
                    <DropdownMenu.Item onclick={() => beginRename(s.id, s.title)}>
                      <Pencil />
                      Rename
                    </DropdownMenu.Item>
                    <DropdownMenu.Separator />
                    <DropdownMenu.Item variant="destructive" onclick={() => deleteSession(s.id)}>
                      <Trash2 />
                      Delete
                    </DropdownMenu.Item>
                  </DropdownMenu.Content>
                </DropdownMenu.Root>
              {/if}
            </SidebarMenuItem>
          {/each}
        </SidebarMenu>
      {/if}
    </SidebarGroup>
  </SidebarContent>

  <SidebarRail />
</Sidebar>
