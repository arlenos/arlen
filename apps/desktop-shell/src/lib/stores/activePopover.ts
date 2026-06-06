import { invoke } from "@tauri-apps/api/core";
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
  | `module:${string}`
  | null;

/// Which popover is currently open. Only one at a time.
export const activePopover = writable<PopoverType>(null);

export function openPopover(type: PopoverType) {
  activePopover.set(type);
  invoke("set_popover_input_region", { expanded: type !== null }).catch(
    () => {},
  );
}

export function closePopover() {
  activePopover.set(null);
  invoke("set_popover_input_region", { expanded: false }).catch(() => {});
}

export function togglePopover(type: PopoverType) {
  activePopover.update((current) => {
    const next = current === type ? null : type;
    invoke("set_popover_input_region", { expanded: next !== null }).catch(
      () => {},
    );
    return next;
  });
}

/// Switch to a different popover on hover (only when one is already open).
export function hoverPopover(type: PopoverType) {
  activePopover.update((current) => {
    if (current === null || current === type) return current;
    invoke("set_popover_input_region", { expanded: true }).catch(() => {});
    return type;
  });
}
