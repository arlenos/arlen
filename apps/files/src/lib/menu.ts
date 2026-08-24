/// The global menu this app publishes into the topbar's menu bar.
///
/// It lives on the frontend because the labels do. The tree used to be built in
/// Rust from English literals and published once at startup, so on a German
/// machine the whole bar read "File Edit View Go Help" over a window whose own
/// chrome was German - and a language switch could never reach it, the menu
/// having been sent before the webview existed.
///
/// Item labels the context menu already names are reused verbatim, so a word
/// like "Rename" is translated in one place and cannot drift between the two
/// surfaces that show it.
import type { Translate } from "@arlen/ui-kit/i18n";
import type { Template } from "$lib/stores/templates";

/// One entry, in the shape `os_sdk::menu::MenuItem` deserializes.
export type MenuItem = {
  label: string;
  action?: string;
  shortcut?: string;
  type: "item" | "separator" | "submenu";
  children?: MenuItem[];
};

/// One top-level group, in the shape `os_sdk::menu::MenuGroup` deserializes.
export type MenuGroup = { label: string; items: MenuItem[] };

const item = (label: string, action: string, shortcut?: string): MenuItem => ({ label, action, shortcut, type: "item" });
const sep = (): MenuItem => ({ label: "", type: "separator" });
const sub = (label: string, children: MenuItem[]): MenuItem => ({
  label,
  type: "submenu",
  children,
});

/// The menu as the reader's language renders it. Pure, so a test can read the
/// labels without a running app or a shell to publish into.
export function appMenuGroups(t: Translate, templates: Template[] = []): MenuGroup[] {
  return [
    {
      label: t("f.gm.file"),
      items: [
        item(t("f.menu.newFolder"), "file.new_folder"),
        item(t("f.gm.newTab"), "file.new_tab"),
        // Only when ~/Templates holds anything: an empty submenu is a promise
        // with nothing behind it.
        ...(templates.length > 0
          ? [sub(t("f.menu.newFromTemplate"), templates.map((tp, i) => item(tp.label, `file.template.${i}`)))]
          : []),
        sep(),
        item(t("f.gm.properties"), "file.properties"),
        sep(),
        item(t("f.gm.closeWindow"), "file.close"),
      ],
    },
    {
      label: t("f.gm.edit"),
      items: [
        item(t("f.gm.undo"), "edit.undo", "Ctrl+Z"),
        sep(),
        item(t("f.menu.cut"), "edit.cut", "Ctrl+X"),
        item(t("f.menu.copy"), "edit.copy", "Ctrl+C"),
        item(t("f.menu.paste"), "edit.paste", "Ctrl+V"),
        sep(),
        item(t("f.menu.rename"), "edit.rename", "F2"),
        item(t("f.menu.moveToTrash"), "edit.trash", "Del"),
        item(t("f.gm.selectAll"), "edit.select_all", "Ctrl+A"),
      ],
    },
    {
      label: t("f.gm.view"),
      items: [
        item(t("f.gm.refresh"), "view.refresh"),
        item(t("f.view.showHidden"), "view.toggle_hidden"),
        sep(),
        sub(t("f.gm.sortBy"), [
          item(t("f.col.name"), "view.sort.name"),
          item(t("f.col.size"), "view.sort.size"),
          item(t("f.filter.type"), "view.sort.type"),
          item(t("f.col.modified"), "view.sort.modified"),
        ]),
      ],
    },
    {
      label: t("f.gm.go"),
      items: [
        item(t("f.place.home"), "go.home"),
        item(t("f.loc.recent"), "go.recent"),
        item(t("f.loc.trash"), "go.trash"),
        sep(),
        item(t("f.gm.parentFolder"), "go.up"),
      ],
    },
    { label: t("f.gm.help"), items: [item(t("f.gm.about"), "help.about")] },
  ];
}

/// Send the menu to the shell. Best-effort, like every other topbar surface:
/// without a shell (or a bus) the app runs on with no menu bar, which is what
/// happened whenever the publish failed before too.
export async function publishAppMenu(t: Translate, templates: Template[] = []): Promise<void> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("publish_menu", { groups: appMenuGroups(t, templates) });
  } catch (e) {
    console.warn("publishAppMenu: the topbar menu was not published:", e);
  }
}
