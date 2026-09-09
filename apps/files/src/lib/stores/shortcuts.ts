/// The app's own verbs, findable from the launcher.
///
/// `shell.shortcuts` is the discoverable-quick-actions surface: the waypointer
/// lists the FOCUSED app's shortcuts, so somebody who opens the launcher over a
/// file manager window can search its actions rather than its files. The shell
/// has drawn that list since it was written and no app in the tree published
/// one, which made a surface promising the app's own actions permanently empty.
///
/// THE SAME ACTION STRINGS THE MENU USES, which `shortcuts-api.md` asks for in
/// as many words: "Same `action` string in both calls keeps the dispatch path
/// unified." So a click here lands in `runMenuAction` - the one dispatcher this
/// app already has - and nothing has to be implemented twice or drift.
///
/// A CURATED SET, not the menu. The menu carries twenty-two entries because a
/// menu is where you look things up; the launcher is where you go when you
/// already know what you want, and a list of twenty-two is not a quick action.
/// These six are the ones with no other one-step route: everything else on the
/// menu is a keystroke, a toolbar button or a right-click away.
///
/// Registered per translator value, like the menu, so a locale switch
/// re-registers - the shell holds no catalogue and renders these labels as they
/// arrive.

import { shortcuts, type Shortcut } from "@arlen/tauri-plugin-shell";
import type { Translate } from "@arlen/ui-kit/i18n";

/// The list as the reader's language renders it. Pure, so a test can read the
/// labels and the action ids without a running app or a shell to publish into.
export function appShortcuts(t: Translate): Shortcut[] {
  return [
    { label: t("f.menu.newFolder"), icon: "folder-plus", action: "file.new_folder", context: [] },
    { label: t("f.gm.newTab"), icon: "plus", action: "file.new_tab", context: [] },
    { label: t("f.view.showHidden"), icon: "eye", action: "view.toggle_hidden", context: [] },
    { label: t("f.place.home"), icon: "home", action: "go.home", context: [] },
    { label: t("f.loc.recent"), icon: "history", action: "go.recent", context: [] },
    { label: t("f.loc.trash"), icon: "trash-2", action: "go.trash", context: [] },
  ];
}

/// Publish the list. Best-effort, like the menu registration: under vite there
/// is no shell relay and the call rejects, and a window whose actions are not in
/// the launcher is still a window.
export async function publishShortcuts(t: Translate): Promise<void> {
  try {
    await shortcuts.register(appShortcuts(t));
  } catch {
    // No shell relay: the launcher lists nothing for this app.
  }
}
