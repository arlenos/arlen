import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";
import { get } from "svelte/store";
import { toast } from "svelte-sonner";
import { t } from "$lib/i18n/messages";
import { writable } from "svelte/store";

export type PopoverType =
  | "quick-settings"
  | "calendar"
  | "notifications"
  | "network"
  | "audio"
  | "battery"
  | "bluetooth"
  | "tray"
  | "layout"
  | "mpris"
  | "undo"
  | `module:${string}`
  | null;

/// Which popover is currently open. Only one at a time.
/// `?popover=<type>` (DEV only) opens one on load so any popover surface is
/// screenshot-able before its live trigger exists - the general form of the
/// `?menumock` pattern.
export const activePopover = writable<PopoverType>(
  !tauriAvailable && typeof location !== "undefined"
    ? ((new URLSearchParams(location.search).get("popover") as PopoverType) ?? null)
    : null
);

/// Widen or narrow the shell's pointer region for an open panel, and say
/// something if it does not take.
///
/// This is not cosmetic and the failure is not benign. The shell is a layer-shell
/// surface whose input region is the bar alone until this call widens it, so a
/// panel drawn without it does not merely fail to respond - **every click inside
/// it falls through to the window underneath**. Someone opens the network panel,
/// clicks a network, and the click lands in whatever app was behind.
///
/// So the open path REVERTS: a panel that cannot receive input must not be left
/// on screen inviting clicks that go somewhere else. Not opening is the safe
/// failure, and it is honest - nothing claimed to have opened.
async function applyInputRegion(expanded: boolean): Promise<boolean> {
  try {
    await invoke("set_popover_input_region", { expanded });
    return true;
  } catch {
    toast.error(get(t)("sh.popover.notOpened"));
    return false;
  }
}

export function openPopover(type: PopoverType) {
  activePopover.set(type);
  void applyInputRegion(type !== null).then((ok) => {
    if (!ok) activePopover.set(null);
  });
}

export function closePopover() {
  activePopover.set(null);
  // The mirror failure has no revert: the panel is gone and the region stays
  // wide, so the shell keeps eating clicks over an area with nothing in it. It
  // still says so, because a top bar that swallows clicks with no panel visible
  // is the least explicable state of the three.
  void applyInputRegion(false);
}

export function togglePopover(type: PopoverType) {
  const next = get(activePopover) === type ? null : type;
  activePopover.set(next);
  void applyInputRegion(next !== null).then((ok) => {
    if (!ok && next !== null) activePopover.set(null);
  });
}

/// Switch to a different popover on hover (only when one is already open).
export function hoverPopover(type: PopoverType) {
  const current = get(activePopover);
  if (current === null || current === type) return;
  activePopover.set(type);
  // A hover switch happens with a panel already open, so the region is already
  // wide and this call is a no-op that only matters if it fails. Falling back to
  // the panel that WAS open keeps a usable one on screen.
  void applyInputRegion(true).then((ok) => {
    if (!ok) activePopover.set(current);
  });
}
