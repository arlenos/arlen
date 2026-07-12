<script lang="ts">
  /// The harness sidebar. The harness IS Chat (harness-redo-plan.md, decided
  /// 11 June), so the sidebar is the chat history: a History group whose
  /// label row carries the new-chat action, a search directly above the list
  /// it narrows, and per-row pin / rename / copy / delete behind a quiet
  /// hover menu. The agent's review feed is reachable through one quiet
  /// Activity entry; it is a secondary view, never a peer mode.
  import { t } from "$lib/i18n/messages";
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
  import {
    Activity,
    Compass,
    Copy,
    Download,
    MoreHorizontal,
    Pencil,
    Pin,
    PinOff,
    Plus,
    Search,
    Share2,
    Trash2,
    Upload,
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
  import { openMint } from "$lib/stores/mint";
  import { openImportChat } from "$lib/stores/importChat";
  import { sessionMatches } from "$lib/search";
  import { groupSessions } from "$lib/session-groups";
  import { conversationToMarkdown, conversationToJson } from "$lib/export";
  import { downloadText } from "$lib/download";

  const path = $derived($page.url.pathname);
  // The chat surface is active unless a secondary surface (the Activity deep
  // list, or the prep-for-this pull view) is open.
  const onChat = $derived(!path.startsWith("/agent") && !path.startsWith("/prep"));

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

  // Export one conversation as a portable JSON envelope (re-importable). The
  // filename is the title, slugged; falls back to a generic name.
  function exportSession(id: string): void {
    const session = $orderedSessions.find((s) => s.id === id);
    if (!session) return;
    const slug = (session.title || "conversation").replace(/[^a-z0-9]+/gi, "-").replace(/^-+|-+$/g, "");
    downloadText(`${slug || "conversation"}.json`, conversationToJson(session), "application/json");
  }

  // Sessions in rail order (pinned first), narrowed by the search. The query
  // matches titles and message content, case-insensitive; empty matches all.
  const filtered = $derived($orderedSessions.filter((s) => sessionMatches(s, query)));
  // The rail in Hollama's shape: a Pinned section, then Today / Yesterday /
  // Previous 7 days / Earlier by creation time. Re-buckets whenever the list
  // or the query changes; an open app crossing midnight without interaction
  // keeps its last buckets, which is fine for a sidebar.
  const groups = $derived(groupSessions(filtered, Date.now()));
</script>

<Sidebar>
  <SidebarContent>
    <SidebarGroup>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton id="harness-new-chat" title={$t("h.sidebar.newChatTitle")} onclick={startNew}>
            <Plus strokeWidth={2} />
            <span>{$t("h.sidebar.newChat")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
      {#if $orderedSessions.length > 0}
        <div class="relative mb-1 mt-1">
          <Search
            size={13}
            strokeWidth={2}
            class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 opacity-50"
          />
          <Input
            id="harness-session-search"
            class="pl-7"
            bind:value={query}
            placeholder={$t("h.sidebar.search")}
            aria-label={$t("h.sidebar.search")}
          />
        </div>
      {/if}
      {#if $orderedSessions.length === 0}
        <p class="px-2 py-2 text-xs leading-relaxed text-sidebar-foreground/55">
          {$t("h.sidebar.emptyChats")}
        </p>
      {:else if filtered.length === 0}
        <p class="px-2 py-2 text-xs text-sidebar-foreground/55">{$t("h.sidebar.noMatch")}</p>
      {/if}
    </SidebarGroup>

    {#each groups as group (group.label)}
      <SidebarGroup class="pt-0">
        <SidebarGroupLabel>{$t(group.label)}</SidebarGroupLabel>
        <SidebarMenu>
          {#each group.sessions as s (s.id)}
            {@render row(s)}
          {/each}
        </SidebarMenu>
      </SidebarGroup>
    {/each}
  </SidebarContent>

  <!-- The secondary, always-visible actions live at the foot, apart from the chat
       history: share a slice of context, import a chat, and the agent's activity
       record. Secondary to Chat, but discoverable. -->
  <SidebarFooter>
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton id="harness-share-context" title={$t("h.sidebar.shareContextTitle")} onclick={openMint}>
          <Share2 strokeWidth={2} />
          <span>{$t("h.sidebar.shareContext")}</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton id="harness-import-chat" title={$t("h.sidebar.importChatTitle")} onclick={openImportChat}>
          <Upload strokeWidth={2} />
          <span>{$t("h.sidebar.importChat")}</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton
          id="harness-prep"
          title={$t("h.sidebar.prepTitle")}
          isActive={path.startsWith("/prep")}
          onclick={() => goto("/prep")}
        >
          <Compass strokeWidth={2} />
          <span>{$t("h.sidebar.prep")}</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton
          id="harness-activity"
          title={$t("h.sidebar.activityTitle")}
          isActive={path.startsWith("/agent")}
          onclick={() => goto("/agent")}
        >
          <Activity strokeWidth={2} />
          <span>{$t("h.sidebar.activity")}</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  </SidebarFooter>

  <SidebarRail />
</Sidebar>

<!-- One conversation row: inline rename when editing, otherwise the title
     button with its hover action menu. Shared across every time group. -->
{#snippet row(s: { id: string; title: string; pinned?: boolean })}
  <SidebarMenuItem>
    {#if editingId === s.id}
      <Input
        bind:value={draft}
        aria-label={$t("h.sidebar.chatName")}
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
          <Pin strokeWidth={1.75} class="ml-auto opacity-50" aria-label={$t("h.sidebar.pinned")} />
        {/if}
      </SidebarMenuButton>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <SidebarMenuAction showOnHover aria-label={$t("h.sidebar.chatActions")} {...props}>
              <MoreHorizontal strokeWidth={2} />
            </SidebarMenuAction>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content side="right" align="start">
          <DropdownMenu.Item onclick={() => togglePinSession(s.id)}>
            {#if s.pinned}
              <PinOff />
              {$t("h.sidebar.unpin")}
            {:else}
              <Pin />
              {$t("h.sidebar.pin")}
            {/if}
          </DropdownMenu.Item>
          <DropdownMenu.Item onclick={() => beginRename(s.id, s.title)}>
            <Pencil />
            {$t("h.sidebar.rename")}
          </DropdownMenu.Item>
          <DropdownMenu.Item onclick={() => copySession(s.id)}>
            <Copy />
            {$t("h.sidebar.copyChat")}
          </DropdownMenu.Item>
          <DropdownMenu.Item onclick={() => exportSession(s.id)}>
            <Download />
            {$t("h.sidebar.exportChat")}
          </DropdownMenu.Item>
          <DropdownMenu.Separator />
          <DropdownMenu.Item variant="destructive" onclick={() => (confirmDeleteId = s.id)}>
            <Trash2 />
            {$t("h.sidebar.delete")}
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    {/if}
  </SidebarMenuItem>
{/snippet}

<ConfirmDialog
  open={confirmDeleteId !== null}
  title={$t("h.sidebar.deleteTitle")}
  message="This removes the chat and its messages. You cannot undo this."
  confirmLabel={$t("h.sidebar.delete")}
  variant="destructive"
  onConfirm={() => {
    if (confirmDeleteId !== null) deleteSession(confirmDeleteId);
    confirmDeleteId = null;
  }}
  onCancel={() => (confirmDeleteId = null)}
/>
