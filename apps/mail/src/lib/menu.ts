/// The mail app's global menu in the shell's top-left menu bar (the knowledge
/// pattern): composing, acting on the open conversation and switching folders,
/// beside the surface's own controls. Labels the toolbar and rail already name
/// are reused verbatim, so a word like "Reply" is translated once. Registered
/// per translator value so a locale switch re-registers; under vite both calls
/// fail silently and the menu is simply absent. Live it needs
/// `arlen-shell:allow-menu-register` / `-unregister` in this app's
/// capabilities file, and the plugin's menu-action return channel - both the
/// same routed seams as the pdf reader's.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { derived, writable } from "svelte/store";
import type { Translate } from "@arlen/ui-kit/i18n";
import { t } from "$lib/i18n/messages";
import { mailboxWritable, mailboxComposes } from "$lib/stores/mailbox";

const APP_ID = "dev.arlen.mail";

/// One entry, in the shape `os_sdk::menu::MenuItem` deserializes.
export type MenuItem = {
  label: string;
  action?: string;
  shortcut?: string;
  type: "item" | "separator";
};
export type MenuGroup = { label: string; items: MenuItem[] };

const item = (label: string, action: string, shortcut?: string): MenuItem => ({
  label,
  action,
  shortcut,
  type: "item",
});
const sep = (): MenuItem => ({ label: "", type: "separator" });

/// The menu as the reader's language renders it. Pure, so a test can read the
/// labels without a running app or a shell to publish into. The Message group
/// is the writes, and it is offered only while the mailbox keeps one; a reader
/// gets Go and nothing that would undo itself at the next start.
export function appMenuGroups(t: Translate, writable = true, composes = writable): MenuGroup[] {
  const message: MenuGroup[] = writable
    ? [
        {
          label: t("ml.menu.message"),
          items: [
            // New message only where a message could go somewhere. A maildir
            // keeps a draft, but sending needs an account and there is none, so
            // the entry is absent live rather than present and unsendable
            // (`mail-app.md`). Reply and Forward answer a message in front of
            // you and stay.
            ...(composes ? [item(t("ml.compose"), "message.new", "Ctrl+N"), sep()] : []),
            item(t("ml.reply"), "message.reply"),
            item(t("ml.forward"), "message.forward"),
            sep(),
            item(t("ml.archive"), "message.archive", "E"),
            item(t("ml.delete"), "message.delete", "Del"),
          ],
        },
      ]
    : [];
  return [
    ...message,
    {
      label: t("ml.menu.go"),
      items: [
        item(t("ml.folder.inbox"), "go.inbox"),
        item(t("ml.folder.sent"), "go.sent"),
        item(t("ml.folder.drafts"), "go.drafts"),
        item(t("ml.folder.archive"), "go.archive"),
        item(t("ml.folder.trash"), "go.trash"),
      ],
    },
  ];
}

/// The action a menu click dispatched; the page consumes and clears it.
export const menuAction = writable<string | null>(null);

/// Register the menu and route dispatched actions into the store. Re-registered
/// on a locale switch and when the mailbox settles, so the Message group comes
/// and goes with what the mailbox can keep.
export async function initAppMenu(): Promise<void> {
  derived([t, mailboxWritable, mailboxComposes], ([tr, writable, composes]) => ({
    tr,
    writable,
    composes,
  })).subscribe(({ tr, writable, composes }) => {
    void invoke("plugin:arlen-shell|menu_register", {
      groups: appMenuGroups(tr, writable, composes),
    }).catch(() => {
      // No shell relay (vite, or the permission seam not landed): absent.
    });
  });
  try {
    await listen<{ app_id: string; action: string }>("arlen://menu-action", ({ payload }) => {
      if (payload.app_id !== APP_ID) return;
      menuAction.set(payload.action);
    });
  } catch {
    // Same seam: without the relay no actions arrive.
  }
}
