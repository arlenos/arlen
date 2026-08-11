/// The harness's global menu, declared against the shell's menu contract
/// (the knowledge app's pattern): app-level commands live in the shell's
/// top-left app menu, not as surface buttons (Tim, 31 Jul). Chat's file-ops
/// (import) move here; the per-conversation actions (export, rename, delete)
/// stay in each row's context menu where their target is. Registration and the
/// action events go through this app's own backend (`src-tauri/src/menu.rs`),
/// which publishes onto the Event Bus and relays clicks back; under vite, with
/// no bus and no shell to reach, both fail and the menu is simply absent.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { goto } from "$app/navigation";
import { newSession } from "$lib/stores/conversation";
import { openImportChat } from "$lib/stores/importChat";

const APP_ID = "dev.arlen.harness";

const MENU = [
  {
    label: "Chat",
    items: [
      { label: "New chat", action: "chat.new", shortcut: "Ctrl+N", type: "item" },
      { label: "", action: "", type: "separator" },
      { label: "Import chat…", action: "chat.import", type: "item" },
    ],
  },
];

/// Register the menu and route dispatched actions into the stores.
export async function initAppMenu(): Promise<void> {
  try {
    await invoke("register_menu", { appId: APP_ID, items: MENU });
  } catch {
    // No shell relay yet: the menu is simply absent.
  }
  try {
    await listen<{ app_id: string; action: string }>("arlen://menu-action", ({ payload }) => {
      if (payload.app_id !== APP_ID) return;
      if (payload.action === "chat.new") {
        newSession();
        void goto("/");
      } else if (payload.action === "chat.import") {
        openImportChat();
      }
    });
  } catch {
    // Same seam: without the relay no actions arrive.
  }
}
