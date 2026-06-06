/// Notifications config store.
///
/// Wraps `~/.config/lunaris/notifications.toml` via the generic
/// `createConfigStore` factory. The schema mirrors
/// `notification-daemon::config::types::Config`.

import { createConfigStore, type ConfigStore } from "./config";

export type DndMode =
  | "off"
  | "priority"
  | "alarms"
  | "total"
  | "scheduled";

export type ScheduleMode = "priority" | "alarms" | "total";

export interface GeneralSection {
  toast_duration_normal?: number;
  toast_duration_high?: number;
  max_visible_toasts?: number;
}

export interface DndScheduleSection {
  start?: string;
  end?: string;
  /// 0 = Monday, 6 = Sunday. Empty array = every day.
  days?: number[];
  mode?: ScheduleMode;
}

export interface DndSection {
  mode?: DndMode;
  /// ISO-8601 UTC timestamp; daemon flips back to Off when reached.
  expires_at?: string;
  schedule?: DndScheduleSection;
  always_suppress?: string[];
  always_allow?: string[];
  suppress_fullscreen?: boolean;
}

export interface HistorySection {
  enabled?: boolean;
  max_age_days?: number;
  max_count?: number;
}

export interface GroupingSection {
  by_app?: boolean;
  stack_similar?: boolean;
  auto_collapse_after?: number;
}

export interface AppOverride {
  enabled?: boolean;
  priority?: "low" | "normal" | "high" | "critical";
  suppress?: boolean;
  toast_duration?: number;
  bypass_dnd?: boolean;
}

export interface NotificationsConfig {
  general?: GeneralSection;
  dnd?: DndSection;
  history?: HistorySection;
  grouping?: GroupingSection;
  apps?: Record<string, AppOverride>;
}

export const notifications: ConfigStore<NotificationsConfig> =
  createConfigStore<NotificationsConfig>("notifications");

/// Localised labels for the German UI (kept inline because there is
/// no i18n layer in the settings app yet — falls direkt mit dem
/// existierenden English-Pattern aus dem Appearance-Panel).
export const DND_MODE_LABELS: Record<DndMode, { title: string; hint: string }> = {
  off: {
    title: "Off",
    hint: "All notifications are shown.",
  },
  priority: {
    title: "Priority Only",
    hint: "Only critical notifications break through.",
  },
  alarms: {
    title: "Alarms Only",
    hint: "Only alarm and reminder notifications.",
  },
  total: {
    title: "Total Silence",
    hint: "Nothing is shown except always-allow apps.",
  },
  scheduled: {
    title: "Scheduled",
    hint: "Follow the time schedule below.",
  },
};
