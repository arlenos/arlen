/// Navigation state: current panel, breadcrumbs, optional scroll target.

import { writable, derived, get } from "svelte/store";
import { goto } from "$app/navigation";

export type PanelId =
  | "appearance"
  | "keyboard"
  | "shortcuts"
  | "mouse"
  | "touchpad"
  | "system-actions"
  | "display"
  | "workspaces"
  | "notifications"
  | "accessibility"
  | "focus"
  | "knowledge"
  | "privacy"
  | "extensions"
  | "about";

export interface PanelMeta {
  id: PanelId;
  title: string;
  icon: string; // Lucide icon name
  enabled: boolean;
  href: string;
}

/// All panels in display order. Disabled ones render greyed out.
/// Sidebar grouping is configured in `AppSidebar.svelte::SECTIONS`.
export const PANELS: PanelMeta[] = [
  // System
  { id: "display", title: "Display", icon: "Monitor", enabled: true, href: "/display" },
  { id: "workspaces", title: "Workspaces & Tiling", icon: "LayoutGrid", enabled: true, href: "/workspaces" },
  { id: "notifications", title: "Notifications", icon: "Bell", enabled: true, href: "/notifications" },
  { id: "about", title: "About", icon: "Info", enabled: true, href: "/about" },

  // Personal
  { id: "appearance", title: "Appearance", icon: "Palette", enabled: true, href: "/appearance" },
  { id: "accessibility", title: "Accessibility", icon: "Accessibility", enabled: true, href: "/accessibility" },
  { id: "focus", title: "Focus Mode", icon: "Crosshair", enabled: true, href: "/focus" },
  { id: "knowledge", title: "Knowledge Graph", icon: "Brain", enabled: true, href: "/knowledge" },

  // Input
  { id: "keyboard", title: "Keyboard", icon: "Keyboard", enabled: true, href: "/keyboard" },
  { id: "shortcuts", title: "Shortcuts", icon: "Command", enabled: true, href: "/keyboard/shortcuts" },
  { id: "mouse", title: "Mouse", icon: "Mouse", enabled: true, href: "/mouse" },
  { id: "touchpad", title: "Touchpad", icon: "SquareMousePointer", enabled: true, href: "/touchpad" },
  { id: "system-actions", title: "System Actions", icon: "Zap", enabled: true, href: "/system-actions" },

  // Apps & Modules
  { id: "extensions", title: "Extensions", icon: "Puzzle", enabled: true, href: "/extensions" },
  { id: "privacy", title: "Privacy", icon: "Shield", enabled: false, href: "/privacy" },
];

interface NavigationState {
  currentPanel: PanelId;
  /// Optional element id to scroll to after navigation (deep-link targets).
  scrollTarget: string | null;
}

const initial: NavigationState = {
  currentPanel: "appearance",
  scrollTarget: null,
};

export const navigation = writable<NavigationState>(initial);

/// Breadcrumb segments for the current route. Panel title first,
/// optional sub-panel segments appended by pages themselves.
export const breadcrumbs = derived(navigation, ($nav) => {
  const panel = PANELS.find((p) => p.id === $nav.currentPanel);
  return panel ? [panel.title] : [];
});

/// Navigate to a panel. Updates both the store and the router.
export async function navigateTo(panel: PanelId, scrollTarget?: string): Promise<void> {
  const meta = PANELS.find((p) => p.id === panel);
  if (!meta || !meta.enabled) return;
  navigation.update((s) => ({
    ...s,
    currentPanel: panel,
    scrollTarget: scrollTarget ?? null,
  }));
  await goto(meta.href);
}

/// Called from +layout.svelte when the route changes, to sync the store.
///
/// Longest-prefix wins so `/keyboard/shortcuts` matches the Shortcuts
/// panel, not Keyboard. A naive `.find(startsWith)` on the registration
/// order would pick whichever panel was listed first.
export function syncFromRoute(pathname: string): void {
  const candidates = PANELS.filter((p) => pathname.startsWith(p.href));
  candidates.sort((a, b) => b.href.length - a.href.length);
  const match = candidates[0];
  if (match) {
    navigation.update((s) =>
      s.currentPanel === match.id ? s : { ...s, currentPanel: match.id }
    );
  }
}

/// Clear the scroll target after consumption (prevents repeat scrolling).
export function consumeScrollTarget(): string | null {
  const current = get(navigation).scrollTarget;
  if (current) {
    navigation.update((s) => ({ ...s, scrollTarget: null }));
  }
  return current;
}
