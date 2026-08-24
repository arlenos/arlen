<script lang="ts">
  /// The folder rail on the kit sidebar (files canon): caps app label in the
  /// head, Compose pinned on top, the five standard folders with icons - which
  /// is what makes the icon-collapse mode legible - and unread counts as quiet
  /// numbers, never dots.
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Archive, Inbox, Send, SquarePen, Trash2, Plus } from "@lucide/svelte";
  import { t } from "$lib/i18n/messages";
  import { folders, envelopes, type FolderKind } from "$lib/stores/mailbox";

  let {
    activeFolder,
    onselect,
    oncompose,
  }: {
    activeFolder: string | null;
    onselect: (id: string) => void;
    oncompose: () => void;
  } = $props();

  const ICONS: Record<FolderKind, typeof Inbox> = {
    inbox: Inbox,
    sent: Send,
    drafts: SquarePen,
    archive: Archive,
    trash: Trash2,
  };

  // Spelled out, not composed with `ml.folder.${kind}`: the key gate reads
  // LITERAL keys and a composed one is invisible to it (wording.ts documents
  // the same rule for the invitation sentences).
  const NAMES: Record<FolderKind, string> = {
    inbox: "ml.folder.inbox",
    sent: "ml.folder.sent",
    drafts: "ml.folder.drafts",
    archive: "ml.folder.archive",
    trash: "ml.folder.trash",
  };

  function unreadIn(folderId: string): number {
    return $envelopes.filter((e) => e.folderId === folderId && e.unread).length;
  }
</script>

<Sidebar collapsible="icon">
  <SidebarContent>
    <SidebarGroup>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton id="mail-compose" onclick={oncompose}>
            <Plus strokeWidth={2} />
            <span>{$t("ml.compose")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarGroup>

    {#if $folders.length > 0}
      <SidebarGroup class="pt-0">
        <SidebarMenu>
          {#each $folders as f (f.id)}
            {@const Icon = ICONS[f.kind]}
            {@const unread = unreadIn(f.id)}
            <SidebarMenuItem>
              <SidebarMenuButton
                id={`folder-${f.id}`}
                isActive={activeFolder === f.id}
                onclick={() => onselect(f.id)}
              >
                <Icon strokeWidth={1.75} />
                <span>{$t(NAMES[f.kind])}</span>
                {#if unread > 0}
                  <span class="ms-auto text-xs tabular-nums text-sidebar-foreground/55 group-data-[collapsible=icon]:hidden"
                    >{unread}</span
                  >
                {/if}
              </SidebarMenuButton>
            </SidebarMenuItem>
          {/each}
        </SidebarMenu>
      </SidebarGroup>
    {/if}
  </SidebarContent>
  <SidebarRail />
</Sidebar>
