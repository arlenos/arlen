/// The app's global menu, declared against the shell's menu contract
/// (`sdk/os-sdk/typescript/shell.ts`, desktop-shell `menu_store.rs`): the
/// timeline's export and delete actions live HERE, in the shell's top-left app
/// menu, not as buttons on the surface (Tim, 31 Jul). Registration and the
/// action events need the shell relay for separate-process apps (a coder
/// seam), so under vite both calls fail silently and the menu simply isn't
/// there; nothing on the surface depends on it existing.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { pendingMenuAction } from "$lib/stores/timeline";

const APP_ID = "org.arlen.knowledge";

const MENU = [
  {
    label: "Timeline",
    items: [
      { label: "Export recorded activity…", action: "timeline.export", type: "item" },
      { label: "", action: "", type: "separator" },
      { label: "Delete today's activity…", action: "timeline.delete-today", type: "item" },
      { label: "Delete everything recorded…", action: "timeline.delete-all", type: "item" },
    ],
  },
];

const ACTIONS: Record<string, "export" | "deleteToday" | "deleteAll"> = {
  "timeline.export": "export",
  "timeline.delete-today": "deleteToday",
  "timeline.delete-all": "deleteAll",
};

/// Register the menu and route dispatched actions into the timeline store.
/// Idempotent enough for one mount; failures are silent by design (no shell).
export async function initAppMenu(): Promise<void> {
  try {
    await invoke("register_menu", { appId: APP_ID, items: MENU });
  } catch {
    // No shell relay yet: the menu is simply absent.
  }
  try {
    await listen<{ app_id: string; action: string }>("arlen://menu-action", ({ payload }) => {
      if (payload.app_id !== APP_ID) return;
      const mapped = ACTIONS[payload.action];
      if (mapped) pendingMenuAction.set(mapped);
    });
  } catch {
    // Same seam: without the relay no actions arrive.
  }
}
