/// The sensing master switches.
///
/// A master switch is the user's statement that a capability is off for
/// everyone, and its only asset is their belief that it worked. Three readers
/// enforce the screen-capture one - this app writes it, the xdg portal refuses
/// the capture portals, the compositor refuses the Wayland capture protocols -
/// and they are held to one vector table so they cannot drift apart.
///
/// Nothing in the interface set it. The switch was enforced everywhere and
/// reachable nowhere, which is the shape where a feature looks finished from
/// every angle except the user's.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One field per switch that exists. Camera and microphone are deliberately
/// absent: there is no portal for either, so a switch would report a protection
/// nothing enforces. They arrive with their portals.
export interface SensingState {
  screenCapture: boolean;
}

/// The switch positions as last read. Defaults to allowed, which is what a
/// system nobody has configured is.
export const sensing = writable<SensingState>({ screenCapture: true });

/// True when the backend could not be reached, so the page can say the switch
/// is not being shown from the real file rather than showing a confident
/// default.
export const sensingUnknown = writable(false);

/// Read the switches.
export async function loadSensing(): Promise<void> {
  try {
    sensing.set(await invoke<SensingState>("settings_sensing_state"));
    sensingUnknown.set(false);
  } catch {
    sensingUnknown.set(true);
  }
}

/// Set the screen-capture switch, optimistically, and put it back if the write
/// is refused - a switch that shows the position it failed to reach is worse
/// than one that snaps back.
export async function setScreenCapture(allowed: boolean): Promise<void> {
  const before = allowed;
  sensing.update((s) => ({ ...s, screenCapture: allowed }));
  try {
    await invoke("settings_sensing_set_screen_capture", { allowed });
  } catch {
    sensing.update((s) => ({ ...s, screenCapture: !before }));
    sensingUnknown.set(true);
  }
}
