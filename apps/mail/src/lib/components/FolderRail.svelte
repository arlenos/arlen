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
  import { folders, envelopes, mailboxWritable, type FolderKind } from "$lib/stores/mailbox";

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
    <!-- The first group keeps a 6px top inset so its row clears the
         window edge by the same gap as the header-bar icons; the row itself
         stays the rail's uniform 32px box - an earlier pass shrank only the
         first box to 28px and it read as broken, smaller and shifted against
         its siblings. -->
    <!-- Compose only while the mailbox keeps a draft; live there is nothing
         that would, so the row is not there rather than there and lying. -->
    {#if $mailboxWritable}
      <SidebarGroup class="pt-1.5">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton id="mail-compose" onclick={oncompose}>
              <Plus strokeWidth={2} />
              <span>{$t("ml.compose")}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarGroup>
    {/if}

    {#if $folders.length > 0}
      <SidebarGroup class={$mailboxWritable ? "pt-0" : "pt-1.5"}>
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
