/// Navigation state: current panel, breadcrumbs, optional scroll target.

import { writable, derived, get } from "svelte/store";
import { goto } from "$app/navigation";

export type PanelId =
  | "appearance"
  | "keyboard"
  | "display"
  | "notifications"
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
export const PANELS: PanelMeta[] = [
  { id: "appearance", title: "Appearance", icon: "Palette", enabled: true, href: "/appearance" },
  { id: "keyboard", title: "Keyboard", icon: "Keyboard", enabled: false, href: "/keyboard" },
  { id: "display", title: "Display", icon: "Monitor", enabled: false, href: "/display" },
  { id: "notifications", title: "Notifications", icon: "Bell", enabled: true, href: "/notifications" },
  { id: "privacy", title: "Privacy", icon: "Shield", enabled: false, href: "/privacy" },
  { id: "extensions", title: "Extensions", icon: "Puzzle", enabled: true, href: "/extensions" },
  { id: "about", title: "About", icon: "Info", enabled: true, href: "/about" },
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

/// Breadcrumb segments for the current route. Always starts with "Settings".
export const breadcrumbs = derived(navigation, ($nav) => {
  const panel = PANELS.find((p) => p.id === $nav.currentPanel);
  if (!panel) return ["Settings"];
  return ["Settings", panel.title];
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
export function syncFromRoute(pathname: string): void {
  const match = PANELS.find((p) => pathname.startsWith(p.href));
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
