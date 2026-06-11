<script lang="ts">
  /// The harness sidebar. The harness IS Chat (harness-redo-plan.md, decided
  /// 11 June), so the sidebar is the chat history: a History group whose
  /// label row carries the new-chat action, a search directly above the list
  /// it narrows, and per-row pin / rename / copy / delete behind a quiet
  /// hover menu. The agent's review feed is reachable through one quiet
  /// Activity entry; it is a secondary view, never a peer mode.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import {
    Sidebar,
    SidebarContent,
    SidebarFooter,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarMenu,
    SidebarMenuAction,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import {
    Activity,
    Copy,
    MoreHorizontal,
    Pencil,
    Pin,
    PinOff,
    Plus,
    Search,
    Trash2,
  } from "@lucide/svelte";
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
  import { conversationToMarkdown } from "$lib/export";

  const onChat = $derived(!$page.url.pathname.startsWith("/agent"));

  let query = $state("");
  // The conversation being renamed inline, and the draft title. `null` when no
  // rename is in progress; double-clicking a title or the row menu opens it.
  let editingId = $state<string | null>(null);
  let draft = $state("");
  // The conversation awaiting delete confirmation; `null` when none.
  let confirmDeleteId = $state<string | null>(null);

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

  function openSession(id: string): void {
    selectSession(id);
    if (!onChat) goto("/");
  }
  function startNew(): void {
    newSession();
    if (!onChat) goto("/");
  }

  // Copy one conversation as a text transcript. Fails silently: a copy that
  // does not land is a minor annoyance, not worth an error surface.
  async function copySession(id: string): Promise<void> {
    const session = $orderedSessions.find((s) => s.id === id);
    if (!session) return;
    const md = conversationToMarkdown(session.messages);
    if (md.length === 0) return;
    try {
      await navigator.clipboard.writeText(md);
    } catch {
      // Clipboard unavailable (locked-down webview); nothing to surface.
    }
  }

  // Sessions in rail order (pinned first), narrowed by the search. The query
  // matches titles and message content, case-insensitive; empty matches all.
  const filtered = $derived($orderedSessions.filter((s) => sessionMatches(s, query)));
</script>

<Sidebar>
  <SidebarContent>
    <SidebarGroup>
      <SidebarGroupLabel>
        <span>History</span>
        <span class="ml-auto">
          <IconAction id="harness-new-chat" label="New chat (Ctrl+N)" onclick={startNew}>
            <Plus size={14} strokeWidth={2} />
          </IconAction>
        </span>
      </SidebarGroupLabel>
      {#if $orderedSessions.length > 0}
        <div class="relative mb-1">
          <Search
            size={13}
            strokeWidth={2}
            class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 opacity-50"
          />
          <Input
            id="harness-session-search"
            class="pl-7"
            bind:value={query}
            placeholder="Search chats"
            aria-label="Search chats"
          />
        </div>
      {/if}
      {#if $orderedSessions.length === 0}
        <p class="px-2 py-2 text-xs leading-relaxed text-sidebar-foreground/55">
          Your chats will show up here.
        </p>
      {:else if filtered.length === 0}
        <p class="px-2 py-2 text-xs text-sidebar-foreground/55">No chats match.</p>
      {:else}
        <SidebarMenu>
          {#each filtered as s (s.id)}
            <SidebarMenuItem>
              {#if editingId === s.id}
                <Input
                  bind:value={draft}
                  aria-label="Chat name"
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
                  isActive={onChat && s.id === $activeSessionId}
                  title={s.title}
                  onclick={() => openSession(s.id)}
                  ondblclick={() => beginRename(s.id, s.title)}
                >
                  <span class="truncate">{s.title}</span>
                  {#if s.pinned}
                    <Pin strokeWidth={1.75} class="ml-auto opacity-50" aria-label="Pinned" />
                  {/if}
                </SidebarMenuButton>
                <DropdownMenu.Root>
                  <DropdownMenu.Trigger>
                    {#snippet child({ props })}
                      <SidebarMenuAction showOnHover aria-label="Chat actions" {...props}>
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
                    <DropdownMenu.Item onclick={() => copySession(s.id)}>
                      <Copy />
                      Copy chat
                    </DropdownMenu.Item>
                    <DropdownMenu.Separator />
                    <DropdownMenu.Item variant="destructive" onclick={() => (confirmDeleteId = s.id)}>
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

  <SidebarFooter>
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={!onChat}
          tooltip="Activity"
          onclick={() => goto("/agent")}
        >
          <Activity strokeWidth={1.75} />
          <span>Activity</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  </SidebarFooter>

  <SidebarRail />
</Sidebar>

<ConfirmDialog
  open={confirmDeleteId !== null}
  title="Delete this chat?"
  message="This removes the chat and its messages. You cannot undo this."
  confirmLabel="Delete"
  variant="destructive"
  onConfirm={() => {
    if (confirmDeleteId !== null) deleteSession(confirmDeleteId);
    confirmDeleteId = null;
  }}
  onCancel={() => (confirmDeleteId = null)}
/>
