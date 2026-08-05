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
import { t } from "$lib/i18n/messages";

const APP_ID = "org.arlen.knowledge";

/// Built per translator value, not once: the menu lives in the shell's process,
/// so a locale switch has to re-register it or the app menu keeps whichever
/// language was active when the app started.
const menuFor = (tr: (k: string) => string) => [
  {
    label: tr("k.menu.timeline"),
    items: [
      { label: tr("k.menu.export"), action: "timeline.export", type: "item" },
      { label: "", action: "", type: "separator" },
      { label: tr("k.menu.deleteToday"), action: "timeline.delete-today", type: "item" },
      { label: tr("k.menu.deleteAll"), action: "timeline.delete-all", type: "item" },
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
  t.subscribe((tr) => {
    void invoke("register_menu", { appId: APP_ID, items: menuFor(tr) }).catch(() => {
      // No shell relay yet: the menu is simply absent.
    });
  });
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
