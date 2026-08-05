/// The global-topbar producer for the terminal: under the Arlen
/// shell the two session-level actions live in the topbar's toolbar
/// slot. Deliberately NO breadcrumb — the cwd lives in the prompt
/// line (terminal.md §4.4), a topbar copy would be the redundancy
/// the header rule exists to avoid.

import { invoke } from "@tauri-apps/api/core";
import { toolbar } from "@arlen/tauri-plugin-shell";
import { tauriAvailable } from "$lib/tauri";
import { newSession } from "$lib/stores/sessions";
import { historyPaletteOpen } from "$lib/stores/history";
import { t } from "$lib/i18n/messages";

// Idempotent: a remount or HMR must not double the actions.
let started = false;

export async function initTopbar(): Promise<void> {
  if (started || !tauriAvailable) return;
  started = true;
  let present = false;
  try {
    present = await invoke<boolean>("shell_present");
  } catch {
    present = false;
  }
  if (!present) return;

  // Pushed on every translator change, not once: these tooltips live in the
  // compositor's topbar, so a locale switch has to re-send them or they keep
  // whichever language was active when the terminal started.
  t.subscribe((tr) => {
    void toolbar.setQuickActions([
      { icon: "plus", action: "new-session", tooltip: tr("term.qa.newSession") },
      { icon: "history", action: "history", tooltip: tr("term.qa.history") },
    ]);
  });
  await toolbar.onAction(({ action }) => {
    if (action === "new-session") {
      void newSession();
    } else if (action === "history") {
      historyPaletteOpen.update((open) => !open);
    }
  });
}
