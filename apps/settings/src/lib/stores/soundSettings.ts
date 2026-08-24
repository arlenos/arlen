/// The notification daemon's sound half (sound-system-plan.md SO-R3): whether
/// cues play at all, which theme, how loud, and per-event silencing. These are
/// daemon config, not theme values - `theme_set_system` deliberately refuses
/// `soundsEnabled`/`soundTheme` (its key table returns None for both), which is
/// why the old System-page controls threw and reverted on every change. This
/// store routes them to their owner instead.
///
/// Mock-vs-live: the two commands are intended contracts the settings bridge
/// does not define yet - `sound_settings` reads the daemon's `SoundConfig`
/// (enabled, theme, volume, overrides), `sound_set { patch }` writes it. Under
/// vite a fixture serves the page with the honest banner; a real session that
/// cannot read shows unavailable rather than invented values.

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";
import { installedSoundThemes, themeCues, type SoundThemeOption } from "./themeSystem";

/// The daemon's sound config as the page renders it. `volume` is 0..1, the
/// daemon's own scale. `overrides` is the daemon's per-event map; the value
/// "disabled" silences that event (the spec's `.disabled`, on the config path),
/// and an empty string clears the entry.
export interface SoundSettings {
  enabled: boolean;
  theme: string;
  volume: number;
  overrides: Record<string, string>;
}

/// The wire name each Settings event row speaks to the daemon. The row keys
/// (`sndNotification` ...) are theme fields; the daemon's override map is keyed
/// by event name.
export const EVENT_WIRE: Record<string, string> = {
  sndNotification: "notification",
  sndError: "error",
  sndWarning: "warning",
  sndAction: "action-completion",
  sndDeviceAdded: "device-added",
  sndDeviceRemoved: "device-removed",
};

interface SoundState {
  /// null means the config could not be read - which is not the same as any
  /// default, so the page says so instead of rendering confident controls.
  settings: SoundSettings | null;
  loading: boolean;
  mocked: boolean;
  unavailable: boolean;
}

const FIXTURE: SoundSettings = {
  enabled: true,
  theme: "arlen",
  volume: 0.8,
  overrides: {},
};

const FIXTURE_THEMES: SoundThemeOption[] = [
  { id: "arlen", name: "Arlen", active: true },
  { id: "arlen-synth", name: "Arlen Synth", active: false },
];

const FIXTURE_CUES = [
  "message-new-instant",
  "dialog-error",
  "dialog-warning",
  "complete",
  "device-added",
  "device-removed",
];

export const sound = writable<SoundState>({
  settings: null,
  loading: false,
  mocked: false,
  unavailable: false,
});

/// True when the last change did not reach the sound service, so the controls
/// went back to what the config really holds.
export const soundWriteFailed = writable(false);

/// Load the daemon's sound config. Live: `sound_settings`; fixture under vite.
export async function loadSound(): Promise<void> {
  sound.update((s) => ({ ...s, loading: true }));
  try {
    const settings = await invoke<SoundSettings>("sound_settings");
    sound.set({ settings, loading: false, mocked: false, unavailable: false });
  } catch {
    if (!tauriAvailable) {
      sound.set({ settings: { ...FIXTURE }, loading: false, mocked: true, unavailable: false });
      return;
    }
    sound.set({ settings: null, loading: false, mocked: false, unavailable: true });
  }
}

/// Change part of the sound config, optimistically. Live: `sound_set`.
export async function patchSound(patch: Partial<SoundSettings>): Promise<void> {
  const before = get(sound).settings;
  if (!before) return;
  sound.update((s) => ({ ...s, settings: s.settings ? { ...s.settings, ...patch } : s.settings }));
  soundWriteFailed.set(false);
  try {
    await invoke("sound_set", { patch });
  } catch {
    if (!tauriAvailable) return; // no host, so no service to ask
    sound.update((s) => ({ ...s, settings: before }));
    soundWriteFailed.set(true);
  }
}

/// Silence one event, or let it play again. Off writes the "disabled" override;
/// on clears the entry, so the theme's cue is what the resolver sees next.
export async function setEventSilenced(rowKey: string, silenced: boolean): Promise<void> {
  const wire = EVENT_WIRE[rowKey];
  const current = get(sound).settings;
  if (!wire || !current) return;
  const overrides = { ...current.overrides };
  if (silenced) overrides[wire] = "disabled";
  else delete overrides[wire];
  await patchSound({ overrides });
}

/// Whether one event row is silenced in the current config.
export function eventSilenced(settings: SoundSettings | null, rowKey: string): boolean {
  const wire = EVENT_WIRE[rowKey];
  return !!settings && !!wire && settings.overrides[wire] === "disabled";
}

/// The installed themes for the picker: the real read where there is a host,
/// the fixture pair (the two a live machine ships) under vite so the surface
/// is designable.
export async function soundThemeOptions(): Promise<SoundThemeOption[]> {
  if (!tauriAvailable) return FIXTURE_THEMES;
  return installedSoundThemes();
}

/// The active theme's cue names for the per-event pickers, same split.
export async function soundCueNames(): Promise<string[]> {
  if (!tauriAvailable) return FIXTURE_CUES;
  return themeCues();
}
