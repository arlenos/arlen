/// Sidebar context: shared state between Provider, Sidebar, Trigger, Inset.
///
/// Svelte 5 reactive context via getContext/setContext with a class that
/// holds $state fields.

import { getContext, setContext } from "svelte";

const KEY = Symbol("sidebar-context");

export const SIDEBAR_COOKIE = "sidebar:state";
export const SIDEBAR_WIDTH = "16rem";
export const SIDEBAR_WIDTH_ICON = "3rem";

export class SidebarState {
  open = $state(true);

  constructor(defaultOpen = true) {
    this.open = defaultOpen;
  }

  toggle(): void {
    this.open = !this.open;
  }
}

/// Create a sidebar state and put it into context. Call from SidebarProvider.
export function setSidebarContext(state: SidebarState): SidebarState {
  return setContext(KEY, state);
}

/// Get the sidebar state from context. Throws if not inside a provider.
export function useSidebar(): SidebarState {
  const ctx = getContext<SidebarState>(KEY);
  if (!ctx) {
    throw new Error("useSidebar must be used inside <SidebarProvider>");
  }
  return ctx;
}
