/// The system-integration theme dimensions: cursor theme + size, icon theme,
/// sound events, and the terminal 16-ANSI palette + fg/bg. Same override model as
/// the other suite pages.
///
/// Mock-vs-live: this is the biggest backend gap - listing installed cursor +
/// icon themes, setting them + an icon generator, the sound event map + playback,
/// and terminal per-slot editing all need coder backend. Fixture-backed here; the
/// option lists are placeholders until the real enumeration lands.

import { writable, derived, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// A selectable option.
///
/// Two fields rather than one because the list mixes kinds. Most entries are the
/// artifact's own name - "Papirus" is what the icon theme is called in any language
/// - while the generic "Default" and "None" choices are ordinary UI text and have to
/// be translated. Overloading `label` to sometimes hold a message id made the id
/// render verbatim in four dropdowns, because the select renders `label` as text and
/// nothing resolved it. A separate field cannot be mistaken for one.
export interface SysOption {
  value: string;
  /// The literal name, for entries that have one.
  label?: string;
  /// A message id, for entries whose text is ours rather than the artifact's.
  labelKey?: string;
}

/// Resolve an option list for display. `t` is the caller's translator, so this stays
/// a plain function rather than a store.
export function sysOptions(
  opts: SysOption[],
  t: (id: string) => string,
): { value: string; label: string }[] {
  return opts.map((o) => ({
    value: o.value,
    label: o.labelKey ? t(o.labelKey) : (o.label ?? o.value),
  }));
}

// A `label` in these lists is an installed package's own name; our words carry a
// `labelKey` instead ("Default", "None"), which is the whole reason the two
// fields exist. Each list is marked separately because a marker covers the one
// declaration under it - saying "from here down" would cover nothing.
//
// i18n-foreign: cursor themes name themselves, on disk and in their own project.
export const CURSOR_THEMES: SysOption[] = [
  { value: "Default", labelKey: "s.sys.default" },
  { value: "Adwaita", label: "Adwaita" },
  { value: "Bibata", label: "Bibata" },
  { value: "Capitaine", label: "Capitaine" },
];

// i18n-foreign: icon sets name themselves, on disk and in their own project.
export const ICON_THEMES: SysOption[] = [
  { value: "Default", labelKey: "s.sys.default" },
  { value: "Papirus", label: "Papirus" },
  { value: "Adwaita", label: "Adwaita" },
  { value: "Numix", label: "Numix" },
  { value: "Tela", label: "Tela" },
];

// The sound themes are no longer a constant: `installedSoundThemes()` below reads
// what the machine actually has. The list that used to sit here named "Chime" and
// "Soft", which exist nowhere, so choosing one wrote a theme the resolver could
// never find while the row showed a confident selection.

// The per-event choices are no longer a constant either: `themeCues()` below
// lists what the active theme actually ships. The list that used to sit here
// named "Bell", "Pop" and "Click", which no theme provides, so choosing one
// wrote a mapping that resolved to nothing and the event fell silent.

/// The four system sound events.
export const SOUND_EVENTS = [
  { key: "sndNotification", label: "s.snd.sndNotification.label", hint: "s.snd.sndNotification.hint" },
  { key: "sndError", label: "s.snd.sndError.label", hint: "s.snd.sndError.hint" },
  { key: "sndWarning", label: "s.snd.sndWarning.label", hint: "s.snd.sndWarning.hint" },
  { key: "sndAction", label: "s.snd.sndAction.label", hint: "s.snd.sndAction.hint" },
  // The two the page could not show until 19 Aug: the daemon has always had the
  // events and the theme schema gained the fields the same day, so they were
  // unconfigurable rather than unwanted.
  { key: "sndDeviceAdded", label: "s.snd.sndDeviceAdded.label", hint: "s.snd.sndDeviceAdded.hint" },
  { key: "sndDeviceRemoved", label: "s.snd.sndDeviceRemoved.label", hint: "s.snd.sndDeviceRemoved.hint" },
];

/// The 16 ANSI slots (normal 0-7, bright 8-15). `label` holds a message KEY,
/// resolved where the swatch renders. The theme and sound-file options above keep
/// their real names - "Papirus" is what the icon theme is called, in any language -
/// and only the generic "Default" and "None" choices carry a key.
export const ANSI_META: { key: string; label: string }[] = [
  { key: "ansi0", label: "s.ansi.ansi0" },
  { key: "ansi1", label: "s.ansi.ansi1" },
  { key: "ansi2", label: "s.ansi.ansi2" },
  { key: "ansi3", label: "s.ansi.ansi3" },
  { key: "ansi4", label: "s.ansi.ansi4" },
  { key: "ansi5", label: "s.ansi.ansi5" },
  { key: "ansi6", label: "s.ansi.ansi6" },
  { key: "ansi7", label: "s.ansi.ansi7" },
  { key: "ansi8", label: "s.ansi.ansi8" },
  { key: "ansi9", label: "s.ansi.ansi9" },
  { key: "ansi10", label: "s.ansi.ansi10" },
  { key: "ansi11", label: "s.ansi.ansi11" },
  { key: "ansi12", label: "s.ansi.ansi12" },
  { key: "ansi13", label: "s.ansi.ansi13" },
  { key: "ansi14", label: "s.ansi.ansi14" },
  { key: "ansi15", label: "s.ansi.ansi15" },
];

/// The active theme's resolved system values (fixture: the house defaults).
export const SYS_DEFAULTS: Record<string, string | number | boolean> = {
  cursorTheme: "Default",
  cursorSize: 24,
  iconTheme: "Default",
  soundsEnabled: true,
  // Matches the notification daemon's own default (`SoundConfig::default`). It
  // said "Chime" until 19 Aug, which named no theme the resolver could find, so
  // the row's default selection was unreachable from the start.
  soundTheme: "arlen",
  sndNotification: "message-new-instant",
  sndError: "dialog-error",
  sndWarning: "dialog-warning",
  sndAction: "complete",
  sndDeviceAdded: "device-added",
  sndDeviceRemoved: "device-removed",
  ansi0: "#1a1d24",
  ansi1: "#dc2626",
  ansi2: "#16a34a",
  ansi3: "#ca8a04",
  ansi4: "#2563eb",
  ansi5: "#a855f7",
  ansi6: "#06b6d4",
  ansi7: "#e6e8ee",
  ansi8: "#3a404d",
  ansi9: "#f87171",
  ansi10: "#4ade80",
  ansi11: "#facc15",
  ansi12: "#60a5fa",
  ansi13: "#c084fc",
  ansi14: "#22d3ee",
  ansi15: "#ffffff",
  termFg: "#e6e8ee",
  termBg: "#0f1115",
};

/// The user's per-field overrides (sparse: only edited fields).
export const overrides = writable<Record<string, string | number | boolean>>({});

/// The effective values: an override wins, else the resolved default.
export const effective = derived(overrides, ($o) => {
  const out: Record<string, string | number | boolean> = { ...SYS_DEFAULTS };
  for (const k of Object.keys($o)) out[k] = $o[k];
  return out;
});

/// Whether a field is overridden.
export function isOverridden(o: Record<string, string | number | boolean>, key: string): boolean {
  return key in o;
}

/// True when a write did not reach the theme file, so the page can say the
/// setting did not take rather than leave the new value sitting on screen.
export const sysWriteFailed = writable(false);

/// Set a field; setting it back to the theme's value clears the override.
///
/// The store moves first so the control does not lag, then the write goes to
/// `theme.toml` through the backend. A refused write puts the previous value
/// back: a row that shows what you picked while the file still says the old
/// thing is the same lie as a fixture, and this one survives a reboot.
export async function setSys(key: string, value: string | number | boolean): Promise<void> {
  const before = get(overrides)[key];
  overrides.update((o) => {
    const next = { ...o };
    if (value === SYS_DEFAULTS[key]) delete next[key];
    else next[key] = value;
    return next;
  });
  if (!tauriAvailable) return; // no host to write through
  try {
    await invoke("theme_set_system", { key, value: String(value) });
    sysWriteFailed.set(false);
  } catch {
    overrides.update((o) => {
      const next = { ...o };
      if (before === undefined) delete next[key];
      else next[key] = before;
      return next;
    });
    sysWriteFailed.set(true);
  }
}

/// Clear a field's override, back to the theme's value. Deletes the key from
/// `theme.toml` too, so the theme's own value is what the resolver sees next time
/// rather than the edit the row no longer shows.
export async function resetSys(key: string): Promise<void> {
  const before = get(overrides)[key];
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
  if (!tauriAvailable) return; // no host to write through
  const path = SYSTEM_PATHS[key];
  if (!path) return; // not a field the theme file holds
  try {
    await invoke("config_reset", { file: "customization", key: path });
    sysWriteFailed.set(false);
  } catch {
    if (before !== undefined) overrides.update((o) => ({ ...o, [key]: before }));
    sysWriteFailed.set(true);
  }
}

/// Where each System field lives in `theme.toml`.
///
/// The same map the backend keeps in `system_key_path`, needed here only because
/// clearing a field goes through the generic `config_reset`, which takes a path
/// rather than a field name. Kept in agreement by a test that asks the backend
/// for every key in this map.
const SYSTEM_PATHS: Record<string, string> = {
  cursorTheme: "cursor.theme",
  cursorSize: "cursor.size",
  iconTheme: "icons.theme",
  sndNotification: "sounds.notification",
  sndError: "sounds.error",
  sndWarning: "sounds.warning",
  sndAction: "sounds.action",
  sndDeviceAdded: "sounds.device_added",
  sndDeviceRemoved: "sounds.device_removed",
  termFg: "terminal.fg",
  termBg: "terminal.bg",
  ansi0: "terminal.ansi.black",
  ansi1: "terminal.ansi.red",
  ansi2: "terminal.ansi.green",
  ansi3: "terminal.ansi.yellow",
  ansi4: "terminal.ansi.blue",
  ansi5: "terminal.ansi.magenta",
  ansi6: "terminal.ansi.cyan",
  ansi7: "terminal.ansi.white",
  ansi8: "terminal.ansi.bright_black",
  ansi9: "terminal.ansi.bright_red",
  ansi10: "terminal.ansi.bright_green",
  ansi11: "terminal.ansi.bright_yellow",
  ansi12: "terminal.ansi.bright_blue",
  ansi13: "terminal.ansi.bright_magenta",
  ansi14: "terminal.ansi.bright_cyan",
  ansi15: "terminal.ansi.bright_white",
};

/// Clear every terminal-palette override at once (the grid's reset-all).
///
/// Through `resetSys` per field rather than by emptying the store, so the file
/// and the page agree afterwards. Clearing the rows on screen while `theme.toml`
/// kept the colours would put the whole palette back at the next launch.
export async function resetTerminal(): Promise<void> {
  const held = Object.keys(get(overrides)).filter(
    (k) => k.startsWith("ansi") || k === "termFg" || k === "termBg",
  );
  for (const k of held) await resetSys(k);
}

/// Read the overrides `theme.toml` currently holds, so the page opens on what
/// the machine is actually set to.
///
/// Without this the page came up on `SYS_DEFAULTS` every time: a value written
/// last week was in the file, in effect, and invisible here - and the reset
/// affordance beside it was dark because the store thought nothing was
/// overridden.
export async function loadSys(): Promise<void> {
  if (!tauriAvailable) return;
  let doc: Record<string, unknown>;
  try {
    doc = (await invoke<Record<string, unknown>>("config_get", {
      file: "customization",
      key: null,
    })) ?? {};
  } catch {
    return; // nothing readable; the page shows the theme's own values
  }
  const found: Record<string, string | number | boolean> = {};
  for (const [key, path] of Object.entries(SYSTEM_PATHS)) {
    const value = path.split(".").reduce<unknown>(
      (node, part) =>
        node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined,
      doc,
    );
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      found[key] = value;
    }
  }
  overrides.set(found);
}

/// What a preview attempt did. The backend answers rather than returning unit,
/// so a button that made no sound can say which kind of nothing happened
/// instead of looking broken.
export type PreviewOutcome = "played" | "silenced" | "not-found" | "no-audio-tool" | "unavailable";

/// Play one cue through the notification daemon's own resolver.
export async function previewSound(name: string): Promise<PreviewOutcome> {
  if (!tauriAvailable) return "unavailable";
  try {
    return (await invoke<string>("sound_preview", { name })) as PreviewOutcome;
  } catch {
    return "unavailable";
  }
}

/// One installed sound theme.
export interface SoundThemeOption {
  id: string;
  name: string;
  active: boolean;
}

/// The themes this machine actually has, for the picker. Empty without a
/// backend, which the page renders as "not measured" rather than as the old
/// invented list.
export async function installedSoundThemes(): Promise<SoundThemeOption[]> {
  if (!tauriAvailable) return [];
  try {
    return await invoke<SoundThemeOption[]>("sound_themes");
  } catch {
    return [];
  }
}

/// The cue names the active theme ships, for the per-event picker.
export async function themeCues(): Promise<string[]> {
  if (!tauriAvailable) return [];
  try {
    return await invoke<string[]>("sound_cues");
  } catch {
    return [];
  }
}
