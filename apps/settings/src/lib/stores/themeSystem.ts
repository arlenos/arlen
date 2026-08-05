/// The system-integration theme dimensions: cursor theme + size, icon theme,
/// sound events, and the terminal 16-ANSI palette + fg/bg. Same override model as
/// the other suite pages.
///
/// Mock-vs-live: this is the biggest backend gap - listing installed cursor +
/// icon themes, setting them + an icon generator, the sound event map + playback,
/// and terminal per-slot editing all need coder backend. Fixture-backed here; the
/// option lists are placeholders until the real enumeration lands.

import { writable, derived } from "svelte/store";

/// A selectable option.
export interface SysOption {
  value: string;
  label: string;
}

export const CURSOR_THEMES: SysOption[] = [
  { value: "Default", label: "s.sys.default" },
  { value: "Adwaita", label: "Adwaita" },
  { value: "Bibata", label: "Bibata" },
  { value: "Capitaine", label: "Capitaine" },
];

export const ICON_THEMES: SysOption[] = [
  { value: "Default", label: "s.sys.default" },
  { value: "Papirus", label: "Papirus" },
  { value: "Adwaita", label: "Adwaita" },
  { value: "Numix", label: "Numix" },
  { value: "Tela", label: "Tela" },
];

export const SOUND_THEMES: SysOption[] = [
  { value: "None", label: "s.sys.none" },
  { value: "Chime", label: "Chime" },
  { value: "Soft", label: "Soft" },
];

export const SOUND_NAMES: SysOption[] = [
  { value: "None", label: "s.sys.none" },
  { value: "Message", label: "Message" },
  { value: "Bell", label: "Bell" },
  { value: "Click", label: "Click" },
  { value: "Pop", label: "Pop" },
  { value: "Chime", label: "Chime" },
];

/// The four system sound events.
export const SOUND_EVENTS = [
  { key: "sndNotification", label: "s.snd.sndNotification.label", hint: "s.snd.sndNotification.hint" },
  { key: "sndError", label: "s.snd.sndError.label", hint: "s.snd.sndError.hint" },
  { key: "sndWarning", label: "s.snd.sndWarning.label", hint: "s.snd.sndWarning.hint" },
  { key: "sndAction", label: "s.snd.sndAction.label", hint: "s.snd.sndAction.hint" },
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
  soundTheme: "Chime",
  sndNotification: "Message",
  sndError: "Bell",
  sndWarning: "Pop",
  sndAction: "Click",
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

/// Set a field; setting it back to the theme's value clears the override.
export function setSys(key: string, value: string | number | boolean): void {
  overrides.update((o) => {
    const next = { ...o };
    if (value === SYS_DEFAULTS[key]) delete next[key];
    else next[key] = value;
    return next;
  });
}

/// Clear a field's override, back to the theme's value.
export function resetSys(key: string): void {
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
}

/// Clear every terminal-palette override at once (the grid's reset-all).
export function resetTerminal(): void {
  overrides.update((o) => {
    const next = { ...o };
    for (const k of Object.keys(next)) {
      if (k.startsWith("ansi") || k === "termFg" || k === "termBg") delete next[k];
    }
    return next;
  });
}
