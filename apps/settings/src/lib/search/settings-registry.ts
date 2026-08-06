/// Settings registry.
///
/// Every user-visible setting is catalogued here once with:
///   * Human-readable title + description
///   * Search keywords
///   * The panel / section it lives in
///   * An optional inline-action definition so Waypointer can modify
///     the setting directly without opening the Settings app
///
/// This file is the single source of truth for both the in-app search
/// and the exported `settings-index.json` that Waypointer reads.

import type { PanelId } from "$lib/stores/navigation";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type InlineActionType = "toggle" | "select" | "slider";

export interface SelectOption {
  value: string;
  /// A message id, for the same reason the entry's other text is: the Waypointer
  /// renders these and resolves them in its own locale.
  labelKey: string;
}

export interface InlineAction {
  type: InlineActionType;
  /// Config file basename (e.g. `"appearance"`) — resolved against
  /// `~/.config/arlen/{file}.toml` at execution time.
  configFile: string;
  /// Dot-notation key within the TOML file.
  configKey: string;
  /// For select actions.
  options?: SelectOption[];
  /// For slider actions.
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
}

export interface SettingDefinition {
  id: string;
  /// Message ids, never prose.
  ///
  /// The exported `settings-index.json` names a catalog and carries these; the
  /// Waypointer resolves them against its own locale. Prose here would be correct
  /// in one language, baked at build time, and a second copy of every string that
  /// can drift from the catalog with nothing to notice - and a third-party app
  /// under the per-app settings plan has to be searchable in a language it never
  /// shipped a snapshot for.
  titleKey: string;
  descKey: string;
  /// One message holding the keywords, comma-separated. They are copy like any
  /// other: somebody searching in German should reach a setting by German words.
  keywordsKey: string;
  panel: PanelId;
  sectionKey: string;
  /// Anchor fragment used in deep links. The frontend scrolls to the
  /// DOM element with `id={anchor}` and briefly highlights it.
  anchor: string;
  inlineAction?: InlineAction;
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

export const SETTINGS_REGISTRY: SettingDefinition[] = [
  // ── Appearance: Theme ──────────────────────────────────────────────
  {
    id: "appearance.theme.mode",
    titleKey: "s.idx.appearance.theme.mode.title",
    descKey: "s.idx.appearance.theme.mode.desc",
    keywordsKey: "s.idx.appearance.theme.mode.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.theme",
    anchor: "theme-mode",
    inlineAction: {
      type: "select",
      configFile: "appearance",
      configKey: "theme.mode",
      options: [
        { value: "light", labelKey: "s.idx.appearance.theme.mode.opt.light" },
        { value: "dark", labelKey: "s.idx.appearance.theme.mode.opt.dark" },
      ],
    },
  },
  {
    id: "appearance.accent",
    titleKey: "s.idx.appearance.accent.title",
    descKey: "s.idx.appearance.accent.desc",
    keywordsKey: "s.idx.appearance.accent.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.theme",
    anchor: "accent-color",
  },

  // ── Quick Settings: layout ────────────────────────────────────────
  {
    id: "quicksettings.layout",
    titleKey: "s.idx.quicksettings.layout.title",
    descKey: "s.idx.quicksettings.layout.desc",
    keywordsKey: "s.idx.quicksettings.layout.keywords",
    panel: "quicksettings",
    sectionKey: "s.idx.section.layout",
    anchor: "qs-layout-list",
  },
  {
    id: "quicksettings.layout.reset",
    titleKey: "s.idx.quicksettings.layout.reset.title",
    descKey: "s.idx.quicksettings.layout.reset.desc",
    keywordsKey: "s.idx.quicksettings.layout.reset.keywords",
    panel: "quicksettings",
    sectionKey: "s.idx.section.layout",
    anchor: "qs-layout-list",
  },

  // ── Printers ───────────────────────────────────────────────────────
  {
    id: "printers.manage",
    titleKey: "s.idx.printers.manage.title",
    descKey: "s.idx.printers.manage.desc",
    keywordsKey: "s.idx.printers.manage.keywords",
    panel: "printers",
    sectionKey: "s.idx.section.printers",
    anchor: "printers-list",
  },
  {
    id: "printers.queue",
    titleKey: "s.idx.printers.queue.title",
    descKey: "s.idx.printers.queue.desc",
    keywordsKey: "s.idx.printers.queue.keywords",
    panel: "printers",
    sectionKey: "s.idx.section.printqueue",
    anchor: "print-queue",
  },
  {
    id: "printers.add",
    titleKey: "s.idx.printers.add.title",
    descKey: "s.idx.printers.add.desc",
    keywordsKey: "s.idx.printers.add.keywords",
    panel: "printers",
    sectionKey: "s.idx.section.addaprinter",
    anchor: "add-printer",
  },

  // ── Appearance: Window ─────────────────────────────────────────────
  {
    id: "appearance.overrides.radius_intensity",
    titleKey: "s.idx.appearance.overrides.radius.intensity.title",
    descKey: "s.idx.appearance.overrides.radius.intensity.desc",
    keywordsKey: "s.idx.appearance.overrides.radius.intensity.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.window",
    anchor: "radius-intensity",
    inlineAction: {
      type: "slider",
      configFile: "appearance",
      configKey: "overrides.radius_intensity",
      min: 0,
      max: 200,
      step: 5,
      unit: "%",
    },
  },
  {
    id: "appearance.window.border_width",
    titleKey: "s.idx.appearance.window.border.width.title",
    descKey: "s.idx.appearance.window.border.width.desc",
    keywordsKey: "s.idx.appearance.window.border.width.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.window",
    anchor: "border-width",
    inlineAction: {
      type: "slider",
      configFile: "appearance",
      configKey: "window.border_width",
      min: 0,
      max: 4,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "appearance.window.gaps",
    titleKey: "s.idx.appearance.window.gaps.title",
    descKey: "s.idx.appearance.window.gaps.desc",
    keywordsKey: "s.idx.appearance.window.gaps.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.window",
    anchor: "gaps",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "layout.inner_gap",
      min: 0,
      max: 24,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "appearance.window.smart_gaps",
    titleKey: "s.idx.appearance.window.smart.gaps.title",
    descKey: "s.idx.appearance.window.smart.gaps.desc",
    keywordsKey: "s.idx.appearance.window.smart.gaps.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.window",
    anchor: "smart-gaps",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "layout.smart_gaps",
    },
  },

  // ── Appearance: Window Borders ─────────────────────────────────────
  {
    id: "appearance.window.border.focused",
    titleKey: "s.idx.appearance.window.border.focused.title",
    descKey: "s.idx.appearance.window.border.focused.desc",
    keywordsKey: "s.idx.appearance.window.border.focused.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.windowborders",
    anchor: "border-focused",
  },
  {
    id: "appearance.window.border.unfocused",
    titleKey: "s.idx.appearance.window.border.unfocused.title",
    descKey: "s.idx.appearance.window.border.unfocused.desc",
    keywordsKey: "s.idx.appearance.window.border.unfocused.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.windowborders",
    anchor: "border-unfocused",
  },

  // ── Appearance: Typography ─────────────────────────────────────────
  {
    id: "appearance.fonts.interface",
    titleKey: "s.idx.appearance.fonts.interface.title",
    descKey: "s.idx.appearance.fonts.interface.desc",
    keywordsKey: "s.idx.appearance.fonts.interface.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.typography",
    anchor: "font-interface",
  },
  {
    id: "appearance.fonts.monospace",
    titleKey: "s.idx.appearance.fonts.monospace.title",
    descKey: "s.idx.appearance.fonts.monospace.desc",
    keywordsKey: "s.idx.appearance.fonts.monospace.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.typography",
    anchor: "font-monospace",
  },
  {
    id: "appearance.fonts.size",
    titleKey: "s.idx.appearance.fonts.size.title",
    descKey: "s.idx.appearance.fonts.size.desc",
    keywordsKey: "s.idx.appearance.fonts.size.keywords",
    panel: "appearance",
    sectionKey: "s.idx.section.typography",
    anchor: "font-size",
    inlineAction: {
      type: "slider",
      configFile: "appearance",
      configKey: "fonts.size",
      min: 12,
      max: 18,
      step: 1,
      unit: "px",
    },
  },

  // ── Notifications: DND ─────────────────────────────────────────────
  {
    id: "notifications.dnd.mode",
    titleKey: "s.idx.notifications.dnd.mode.title",
    descKey: "s.idx.notifications.dnd.mode.desc",
    keywordsKey: "s.idx.notifications.dnd.mode.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.donotdisturb",
    anchor: "dnd-mode",
    inlineAction: {
      type: "select",
      configFile: "notifications",
      configKey: "dnd.mode",
      options: [
        { value: "off", labelKey: "s.idx.notifications.dnd.mode.opt.off" },
        { value: "priority", labelKey: "s.idx.notifications.dnd.mode.opt.priority" },
        { value: "alarms", labelKey: "s.idx.notifications.dnd.mode.opt.alarms" },
        { value: "total", labelKey: "s.idx.notifications.dnd.mode.opt.total" },
        { value: "scheduled", labelKey: "s.idx.notifications.dnd.mode.opt.scheduled" },
      ],
    },
  },
  {
    id: "notifications.dnd.suppress_fullscreen",
    titleKey: "s.idx.notifications.dnd.suppress.fullscreen.title",
    descKey: "s.idx.notifications.dnd.suppress.fullscreen.desc",
    keywordsKey: "s.idx.notifications.dnd.suppress.fullscreen.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.donotdisturb",
    anchor: "suppress-fullscreen",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "dnd.suppress_fullscreen",
    },
  },

  // ── Notifications: Timing ──────────────────────────────────────────
  {
    id: "notifications.general.toast_duration_normal",
    titleKey: "s.idx.notifications.general.toast.duration.normal.title",
    descKey: "s.idx.notifications.general.toast.duration.normal.desc",
    keywordsKey: "s.idx.notifications.general.toast.duration.normal.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.timing",
    anchor: "toast-duration-normal",
    inlineAction: {
      type: "slider",
      configFile: "notifications",
      configKey: "general.toast_duration_normal",
      min: 1000,
      max: 15000,
      step: 500,
      unit: "ms",
    },
  },
  {
    id: "notifications.general.toast_duration_high",
    titleKey: "s.idx.notifications.general.toast.duration.high.title",
    descKey: "s.idx.notifications.general.toast.duration.high.desc",
    keywordsKey: "s.idx.notifications.general.toast.duration.high.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.timing",
    anchor: "toast-duration-high",
  },
  {
    id: "notifications.general.max_visible_toasts",
    titleKey: "s.idx.notifications.general.max.visible.toasts.title",
    descKey: "s.idx.notifications.general.max.visible.toasts.desc",
    keywordsKey: "s.idx.notifications.general.max.visible.toasts.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.timing",
    anchor: "max-visible",
  },

  // ── Notifications: Toast Appearance ────────────────────────────────
  {
    id: "notifications.toast.position",
    titleKey: "s.idx.notifications.toast.position.title",
    descKey: "s.idx.notifications.toast.position.desc",
    keywordsKey: "s.idx.notifications.toast.position.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.toastappearance",
    anchor: "toast-position",
    inlineAction: {
      type: "select",
      configFile: "shell",
      configKey: "toast.position",
      options: [
        { value: "top-right", labelKey: "s.idx.notifications.toast.position.opt.topright" },
        { value: "top-left", labelKey: "s.idx.notifications.toast.position.opt.topleft" },
        { value: "top-center", labelKey: "s.idx.notifications.toast.position.opt.topcenter" },
        { value: "bottom-right", labelKey: "s.idx.notifications.toast.position.opt.bottomright" },
        { value: "bottom-left", labelKey: "s.idx.notifications.toast.position.opt.bottomleft" },
      ],
    },
  },
  {
    id: "notifications.toast.animation",
    titleKey: "s.idx.notifications.toast.animation.title",
    descKey: "s.idx.notifications.toast.animation.desc",
    keywordsKey: "s.idx.notifications.toast.animation.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.toastappearance",
    anchor: "toast-animation",
    inlineAction: {
      type: "select",
      configFile: "shell",
      configKey: "toast.animation",
      options: [
        { value: "slide", labelKey: "s.idx.notifications.toast.animation.opt.slide" },
        { value: "fade", labelKey: "s.idx.notifications.toast.animation.opt.fade" },
        { value: "none", labelKey: "s.idx.notifications.toast.animation.opt.none" },
      ],
    },
  },

  // ── Notifications: Grouping ────────────────────────────────────────
  {
    id: "notifications.grouping.by_app",
    titleKey: "s.idx.notifications.grouping.by.app.title",
    descKey: "s.idx.notifications.grouping.by.app.desc",
    keywordsKey: "s.idx.notifications.grouping.by.app.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.grouping",
    anchor: "group-by-app",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "grouping.by_app",
    },
  },
  {
    id: "notifications.grouping.stack_similar",
    titleKey: "s.idx.notifications.grouping.stack.similar.title",
    descKey: "s.idx.notifications.grouping.stack.similar.desc",
    keywordsKey: "s.idx.notifications.grouping.stack.similar.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.grouping",
    anchor: "stack-similar",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "grouping.stack_similar",
    },
  },

  // ── Notifications: History ─────────────────────────────────────────
  {
    id: "notifications.history.enabled",
    titleKey: "s.idx.notifications.history.enabled.title",
    descKey: "s.idx.notifications.history.enabled.desc",
    keywordsKey: "s.idx.notifications.history.enabled.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.history",
    anchor: "history-enabled",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "history.enabled",
    },
  },
  {
    id: "notifications.history.max_age_days",
    titleKey: "s.idx.notifications.history.max.age.days.title",
    descKey: "s.idx.notifications.history.max.age.days.desc",
    keywordsKey: "s.idx.notifications.history.max.age.days.keywords",
    panel: "notifications",
    sectionKey: "s.idx.section.history",
    anchor: "history-max-age",
  },

  // ── Keyboard: layout ───────────────────────────────────────────────
  {
    id: "keyboard.layout",
    titleKey: "s.idx.keyboard.layout.title",
    descKey: "s.idx.keyboard.layout.desc",
    keywordsKey: "s.idx.keyboard.layout.keywords",
    panel: "keyboard",
    sectionKey: "s.idx.section.layout",
    anchor: "search",
  },
  {
    id: "keyboard.repeat",
    titleKey: "s.idx.keyboard.repeat.title",
    descKey: "s.idx.keyboard.repeat.desc",
    keywordsKey: "s.idx.keyboard.repeat.keywords",
    panel: "keyboard",
    sectionKey: "s.idx.section.keyrepeat",
    anchor: "search",
  },

  // ── Keyboard: shortcuts ────────────────────────────────────────────
  {
    id: "shortcuts.all",
    titleKey: "s.idx.shortcuts.all.title",
    descKey: "s.idx.shortcuts.all.desc",
    keywordsKey: "s.idx.shortcuts.all.keywords",
    panel: "shortcuts",
    sectionKey: "s.idx.section.shortcuts",
    anchor: "search",
  },
  {
    id: "shortcuts.reset_all",
    titleKey: "s.idx.shortcuts.reset.all.title",
    descKey: "s.idx.shortcuts.reset.all.desc",
    keywordsKey: "s.idx.shortcuts.reset.all.keywords",
    panel: "shortcuts",
    sectionKey: "s.idx.section.shortcuts",
    anchor: "search",
  },
  {
    id: "shortcuts.workspace",
    titleKey: "s.idx.shortcuts.workspace.title",
    descKey: "s.idx.shortcuts.workspace.desc",
    keywordsKey: "s.idx.shortcuts.workspace.keywords",
    panel: "shortcuts",
    sectionKey: "s.idx.section.workspaces",
    anchor: "cat-workspace",
  },
  {
    id: "shortcuts.tiling",
    titleKey: "s.idx.shortcuts.tiling.title",
    descKey: "s.idx.shortcuts.tiling.desc",
    keywordsKey: "s.idx.shortcuts.tiling.keywords",
    panel: "shortcuts",
    sectionKey: "s.idx.section.tiling",
    anchor: "cat-tiling",
  },

  // ── Mouse ──────────────────────────────────────────────────────────
  {
    id: "mouse.acceleration",
    titleKey: "s.idx.mouse.acceleration.title",
    descKey: "s.idx.mouse.acceleration.desc",
    keywordsKey: "s.idx.mouse.acceleration.keywords",
    panel: "mouse",
    sectionKey: "s.idx.section.behavior",
    anchor: "mouse-acceleration",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "mouse.acceleration",
      min: -1,
      max: 1,
      step: 0.1,
    },
  },
  {
    id: "mouse.natural_scroll",
    titleKey: "s.idx.mouse.natural.scroll.title",
    descKey: "s.idx.mouse.natural.scroll.desc",
    keywordsKey: "s.idx.mouse.natural.scroll.keywords",
    panel: "mouse",
    sectionKey: "s.idx.section.behavior",
    anchor: "mouse-natural-scroll",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "mouse.natural_scroll",
    },
  },
  {
    id: "mouse.left_handed",
    titleKey: "s.idx.mouse.left.handed.title",
    descKey: "s.idx.mouse.left.handed.desc",
    keywordsKey: "s.idx.mouse.left.handed.keywords",
    panel: "mouse",
    sectionKey: "s.idx.section.behavior",
    anchor: "mouse-left-handed",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "mouse.left_handed",
    },
  },

  // ── Touchpad ───────────────────────────────────────────────────────
  {
    id: "touchpad.tap_to_click",
    titleKey: "s.idx.touchpad.tap.to.click.title",
    descKey: "s.idx.touchpad.tap.to.click.desc",
    keywordsKey: "s.idx.touchpad.tap.to.click.keywords",
    panel: "touchpad",
    sectionKey: "s.idx.section.clicking",
    anchor: "touchpad-tap",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.tap_to_click",
    },
  },
  {
    id: "touchpad.natural_scroll",
    titleKey: "s.idx.touchpad.natural.scroll.title",
    descKey: "s.idx.touchpad.natural.scroll.desc",
    keywordsKey: "s.idx.touchpad.natural.scroll.keywords",
    panel: "touchpad",
    sectionKey: "s.idx.section.scrolling",
    anchor: "touchpad-natural-scroll",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.natural_scroll",
    },
  },
  {
    id: "touchpad.two_finger_scroll",
    titleKey: "s.idx.touchpad.two.finger.scroll.title",
    descKey: "s.idx.touchpad.two.finger.scroll.desc",
    keywordsKey: "s.idx.touchpad.two.finger.scroll.keywords",
    panel: "touchpad",
    sectionKey: "s.idx.section.scrolling",
    anchor: "touchpad-two-finger",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.two_finger_scroll",
    },
  },
  {
    id: "touchpad.disable_while_typing",
    titleKey: "s.idx.touchpad.disable.while.typing.title",
    descKey: "s.idx.touchpad.disable.while.typing.desc",
    keywordsKey: "s.idx.touchpad.disable.while.typing.keywords",
    panel: "touchpad",
    sectionKey: "s.idx.section.clicking",
    anchor: "touchpad-dwt",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.disable_while_typing",
    },
  },
  {
    id: "touchpad.acceleration",
    titleKey: "s.idx.touchpad.acceleration.title",
    descKey: "s.idx.touchpad.acceleration.desc",
    keywordsKey: "s.idx.touchpad.acceleration.keywords",
    panel: "touchpad",
    sectionKey: "s.idx.section.pointer",
    anchor: "touchpad-acceleration",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "touchpad.acceleration",
      min: -1,
      max: 1,
      step: 0.1,
    },
  },

  // ── Workspaces & Tiling (Sprint B) ─────────────────────────────────
  {
    id: "workspaces.layout",
    titleKey: "s.idx.workspaces.layout.title",
    descKey: "s.idx.workspaces.layout.desc",
    keywordsKey: "s.idx.workspaces.layout.keywords",
    panel: "workspaces",
    sectionKey: "s.idx.section.workspacelayout",
    anchor: "workspace-layout",
    inlineAction: {
      type: "select",
      configFile: "compositor",
      configKey: "workspaces.workspace_layout",
      options: [
        { value: "Horizontal", labelKey: "s.idx.workspaces.layout.opt.horizontal" },
        { value: "Vertical", labelKey: "s.idx.workspaces.layout.opt.vertical" },
      ],
    },
  },
  {
    id: "tiling.inner_gap",
    titleKey: "s.idx.tiling.inner.gap.title",
    descKey: "s.idx.tiling.inner.gap.desc",
    keywordsKey: "s.idx.tiling.inner.gap.keywords",
    panel: "workspaces",
    sectionKey: "s.idx.section.tiling",
    anchor: "inner-gap",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "layout.inner_gap",
      min: 0,
      max: 32,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "tiling.outer_gap",
    titleKey: "s.idx.tiling.outer.gap.title",
    descKey: "s.idx.tiling.outer.gap.desc",
    keywordsKey: "s.idx.tiling.outer.gap.keywords",
    panel: "workspaces",
    sectionKey: "s.idx.section.tiling",
    anchor: "outer-gap",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "layout.outer_gap",
      min: 0,
      max: 32,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "tiling.smart_gaps",
    titleKey: "s.idx.tiling.smart.gaps.title",
    descKey: "s.idx.tiling.smart.gaps.desc",
    keywordsKey: "s.idx.tiling.smart.gaps.keywords",
    panel: "workspaces",
    sectionKey: "s.idx.section.tiling",
    anchor: "smart-gaps",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "layout.smart_gaps",
    },
  },
  {
    id: "tiling.tiled_headers",
    titleKey: "s.idx.tiling.tiled.headers.title",
    descKey: "s.idx.tiling.tiled.headers.desc",
    keywordsKey: "s.idx.tiling.tiled.headers.keywords",
    panel: "workspaces",
    sectionKey: "s.idx.section.tiling",
    anchor: "tiled-headers",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "layout.tiled_headers",
    },
  },
  {
    id: "tiling.window_rules",
    titleKey: "s.idx.tiling.window.rules.title",
    descKey: "s.idx.tiling.window.rules.desc",
    keywordsKey: "s.idx.tiling.window.rules.keywords",
    panel: "workspaces",
    sectionKey: "s.idx.section.windowrules",
    anchor: "window-rules",
  },

  // ── System Actions (Sprint B) ──────────────────────────────────────
  // No inlineActions — system-action commands are free-form strings
  // that don't fit the toggle/select/slider model. Settings users
  // who want to change them open the panel via deepLink.
  {
    id: "system-actions.volume",
    titleKey: "s.idx.system-actions.volume.title",
    descKey: "s.idx.system-actions.volume.desc",
    keywordsKey: "s.idx.system-actions.volume.keywords",
    panel: "system-actions",
    sectionKey: "s.idx.section.volume",
    anchor: "action-VolumeRaise",
  },
  {
    id: "system-actions.brightness",
    titleKey: "s.idx.system-actions.brightness.title",
    descKey: "s.idx.system-actions.brightness.desc",
    keywordsKey: "s.idx.system-actions.brightness.keywords",
    panel: "system-actions",
    sectionKey: "s.idx.section.brightness",
    anchor: "action-BrightnessUp",
  },
  {
    id: "system-actions.media",
    titleKey: "s.idx.system-actions.media.title",
    descKey: "s.idx.system-actions.media.desc",
    keywordsKey: "s.idx.system-actions.media.keywords",
    panel: "system-actions",
    sectionKey: "s.idx.section.media",
    anchor: "action-PlayPause",
  },
  {
    id: "system-actions.system",
    titleKey: "s.idx.system-actions.system.title",
    descKey: "s.idx.system-actions.system.desc",
    keywordsKey: "s.idx.system-actions.system.keywords",
    panel: "system-actions",
    sectionKey: "s.idx.section.system",
    anchor: "action-LockScreen",
  },

  // ── Accessibility (Sprint C) ───────────────────────────────────────
  {
    id: "accessibility.zoom.shortcuts",
    titleKey: "s.idx.accessibility.zoom.shortcuts.title",
    descKey: "s.idx.accessibility.zoom.shortcuts.desc",
    keywordsKey: "s.idx.accessibility.zoom.shortcuts.keywords",
    panel: "accessibility",
    sectionKey: "s.idx.section.screenmagnifier",
    anchor: "zoom-shortcuts",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "accessibility_zoom.enable_mouse_zoom_shortcuts",
    },
  },
  {
    id: "accessibility.zoom.increment",
    titleKey: "s.idx.accessibility.zoom.increment.title",
    descKey: "s.idx.accessibility.zoom.increment.desc",
    keywordsKey: "s.idx.accessibility.zoom.increment.keywords",
    panel: "accessibility",
    sectionKey: "s.idx.section.screenmagnifier",
    anchor: "zoom-increment",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "accessibility_zoom.increment",
      min: 5,
      max: 200,
      step: 5,
      unit: "%",
    },
  },
  {
    id: "accessibility.zoom.movement",
    titleKey: "s.idx.accessibility.zoom.movement.title",
    descKey: "s.idx.accessibility.zoom.movement.desc",
    keywordsKey: "s.idx.accessibility.zoom.movement.keywords",
    panel: "accessibility",
    sectionKey: "s.idx.section.screenmagnifier",
    anchor: "zoom-movement",
    inlineAction: {
      type: "select",
      configFile: "compositor",
      configKey: "accessibility_zoom.view_moves",
      options: [
        { value: "Continuously", labelKey: "s.idx.accessibility.zoom.movement.opt.continuously" },
        { value: "OnEdge", labelKey: "s.idx.accessibility.zoom.movement.opt.onedge" },
        { value: "Centered", labelKey: "s.idx.accessibility.zoom.movement.opt.centered" },
      ],
    },
  },
  {
    id: "accessibility.zoom.start_on_login",
    titleKey: "s.idx.accessibility.zoom.start.on.login.title",
    descKey: "s.idx.accessibility.zoom.start.on.login.desc",
    keywordsKey: "s.idx.accessibility.zoom.start.on.login.keywords",
    panel: "accessibility",
    sectionKey: "s.idx.section.screenmagnifier",
    anchor: "zoom-start-on-login",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "accessibility_zoom.start_on_login",
    },
  },
  {
    id: "accessibility.invert",
    titleKey: "s.idx.accessibility.invert.title",
    descKey: "s.idx.accessibility.invert.desc",
    keywordsKey: "s.idx.accessibility.invert.keywords",
    panel: "accessibility",
    sectionKey: "s.idx.section.colorfilters",
    anchor: "invert-colors",
  },
  {
    id: "accessibility.color_blindness",
    titleKey: "s.idx.accessibility.color.blindness.title",
    descKey: "s.idx.accessibility.color.blindness.desc",
    keywordsKey: "s.idx.accessibility.color.blindness.keywords",
    panel: "accessibility",
    sectionKey: "s.idx.section.colorfilters",
    anchor: "color-blindness-filter",
  },

  // ── Focus Mode (Sprint C) ──────────────────────────────────────────
  {
    id: "focus.show_project_name",
    titleKey: "s.idx.focus.show.project.name.title",
    descKey: "s.idx.focus.show.project.name.desc",
    keywordsKey: "s.idx.focus.show.project.name.keywords",
    panel: "focus",
    sectionKey: "s.idx.section.topbarindicator",
    anchor: "focus-show-project-name",
    inlineAction: {
      type: "toggle",
      configFile: "shell",
      configKey: "focus_settings.show_project_name",
    },
  },
  {
    id: "focus.suppressed_apps",
    titleKey: "s.idx.focus.suppressed.apps.title",
    descKey: "s.idx.focus.suppressed.apps.desc",
    keywordsKey: "s.idx.focus.suppressed.apps.keywords",
    panel: "focus",
    sectionKey: "s.idx.section.defaultsuppressedapps",
    anchor: "focus-suppressed-apps",
  },
  {
    id: "knowledge.promote_threshold",
    titleKey: "s.idx.knowledge.promote.threshold.title",
    descKey: "s.idx.knowledge.promote.threshold.desc",
    keywordsKey: "s.idx.knowledge.promote.threshold.keywords",
    panel: "knowledge",
    sectionKey: "s.idx.section.projectdetection",
    anchor: "kg-promote",
    inlineAction: {
      type: "slider",
      configFile: "graph",
      configKey: "projects.auto_promote_threshold",
      min: 1,
      max: 20,
      step: 1,
      unit: "files",
    },
  },
  {
    id: "knowledge.watch_dirs",
    titleKey: "s.idx.knowledge.watch.dirs.title",
    descKey: "s.idx.knowledge.watch.dirs.desc",
    keywordsKey: "s.idx.knowledge.watch.dirs.keywords",
    panel: "knowledge",
    sectionKey: "s.idx.section.projectdetection",
    anchor: "kg-watch-dirs",
  },
  {
    id: "knowledge.max_depth",
    titleKey: "s.idx.knowledge.max.depth.title",
    descKey: "s.idx.knowledge.max.depth.desc",
    keywordsKey: "s.idx.knowledge.max.depth.keywords",
    panel: "knowledge",
    sectionKey: "s.idx.section.projectdetection",
    anchor: "kg-max-depth",
    inlineAction: {
      type: "slider",
      configFile: "graph",
      configKey: "projects.max_depth",
      min: 1,
      max: 10,
      step: 1,
      unit: "levels",
    },
  },

  // ── Knowledge Graph (Sprint C) ─────────────────────────────────────
  {
    id: "knowledge.app",
    titleKey: "s.idx.knowledge.app.title",
    descKey: "s.idx.knowledge.app.desc",
    keywordsKey: "s.idx.knowledge.app.keywords",
    panel: "knowledge",
    sectionKey: "s.idx.section.knowledgeapp",
    anchor: "kg-app-link",
  },
  {
    id: "knowledge.stats",
    titleKey: "s.idx.knowledge.stats.title",
    descKey: "s.idx.knowledge.stats.desc",
    keywordsKey: "s.idx.knowledge.stats.keywords",
    panel: "knowledge",
    sectionKey: "s.idx.section.stats",
    anchor: "kg-daemon-status",
  },

  // ── Display (Sprint D coverage) ────────────────────────────────────
  // Display panel uses bespoke layout components that don't yet
  // declare per-control DOM ids. We index the panel itself so
  // search by "monitor"/"resolution"/"display" surfaces it; deep
  // linking to specific controls is a follow-up that needs to be
  // paired with `id={anchor}` props on each Row (Codex Sprint D
  // review MEDIUM 2).
  {
    id: "display.panel",
    titleKey: "s.idx.display.panel.title",
    descKey: "s.idx.display.panel.desc",
    keywordsKey: "s.idx.display.panel.keywords",
    panel: "display",
    sectionKey: "s.idx.section.display",
    anchor: "",
  },

  // ── Keyboard extends (Sprint D coverage) ───────────────────────────
  // Same caveat as Display — Keyboard panel uses custom layout-
  // editor components without per-row ids. Single panel entry
  // covers search; per-row deepLinks are a follow-up.
  {
    id: "keyboard.panel",
    titleKey: "s.idx.keyboard.panel.title",
    descKey: "s.idx.keyboard.panel.desc",
    keywordsKey: "s.idx.keyboard.panel.keywords",
    panel: "keyboard",
    sectionKey: "s.idx.section.keyboard",
    anchor: "",
  },

  // ── App access (the permission browser) ────────────────────────────
  {
    id: "privacy.panel",
    titleKey: "s.idx.privacy.panel.title",
    descKey: "s.idx.privacy.panel.desc",
    keywordsKey: "s.idx.privacy.panel.keywords",
    panel: "privacy",
    sectionKey: "s.idx.section.privacy",
    anchor: "",
  },

  // ── Extensions (Sprint D coverage) ─────────────────────────────────
  {
    id: "extensions.panel",
    titleKey: "s.idx.extensions.panel.title",
    descKey: "s.idx.extensions.panel.desc",
    keywordsKey: "s.idx.extensions.panel.keywords",
    panel: "extensions",
    sectionKey: "s.idx.section.modules",
    anchor: "",
  },

  // ── About (Sprint D coverage) ──────────────────────────────────────
  {
    id: "about.version",
    titleKey: "s.idx.about.version.title",
    descKey: "s.idx.about.version.desc",
    keywordsKey: "s.idx.about.version.keywords",
    panel: "about",
    sectionKey: "s.idx.section.arlenos",
    anchor: "arlen-version",
  },
  {
    id: "about.daemons",
    titleKey: "s.idx.about.daemons.title",
    descKey: "s.idx.about.daemons.desc",
    keywordsKey: "s.idx.about.daemons.keywords",
    panel: "about",
    sectionKey: "s.idx.section.daemons",
    anchor: "daemon-knowledge-graph",
  },

  // ── AI (Phase 9-α S7) ──────────────────────────────────────────────
  {
    id: "ai.enable",
    titleKey: "s.idx.ai.enable.title",
    descKey: "s.idx.ai.enable.desc",
    keywordsKey: "s.idx.ai.enable.keywords",
    panel: "ai",
    sectionKey: "s.idx.section.ailayer",
    anchor: "ai-enable",
  },
  {
    id: "ai.provider",
    titleKey: "s.idx.ai.provider.title",
    descKey: "s.idx.ai.provider.desc",
    keywordsKey: "s.idx.ai.provider.keywords",
    panel: "ai",
    sectionKey: "s.idx.section.provider",
    anchor: "ai-provider",
  },
  {
    id: "ai.status",
    titleKey: "s.idx.ai.status.title",
    descKey: "s.idx.ai.status.desc",
    keywordsKey: "s.idx.ai.status.keywords",
    panel: "ai",
    sectionKey: "s.idx.section.status",
    anchor: "ai-daemon-status",
  },
  {
    id: "ai.behaviours",
    titleKey: "s.idx.ai.behaviours.title",
    descKey: "s.idx.ai.behaviours.desc",
    keywordsKey: "s.idx.ai.behaviours.keywords",
    panel: "ai",
    sectionKey: "s.idx.section.behaviours",
    anchor: "ai-behaviours",
  },
  {
    id: "ai.executor",
    titleKey: "s.idx.ai.executor.title",
    descKey: "s.idx.ai.executor.desc",
    keywordsKey: "s.idx.ai.executor.keywords",
    panel: "ai",
    sectionKey: "s.idx.section.execution",
    anchor: "ai-executor-live",
  },

  // Privacy panel intentionally NOT in the registry: it's
  // disabled in navigation until Phase 8 ships the
  // permission-management UI. Indexing it would surface
  // misleading hits in Waypointer search (Codex Sprint D review
  // HIGH 1). The placeholder page still renders for direct-URL
  // visitors so the architecture link stays reachable.

  // Mouse + Touchpad: existing entries already cover the well-
  // known anchors (acceleration, tap-to-click, …); per-control
  // additions for middle-click-emulation, tap-drag-lock, etc.
  // would point to non-existent ids on the existing pages. Drop
  // the broken extras until the pages opt into id={anchor} (Codex
  // Sprint D review MEDIUM 2).
];
