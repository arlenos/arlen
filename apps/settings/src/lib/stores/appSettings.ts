/// The per-app settings state (per-app-settings-plan.md): the broker-served page
/// (`app_settings_page`), writes through the broker (`app_settings_write` - the
/// broker validates against schema+scope, the page never asserts), the raw-TOML
/// escape hatch, and dynamic option resolution. All four commands are LIVE in
/// src-tauri (PAS-0..7); under vite the fixture stands in - a demo schema that
/// exercises every declared type so the renderer is fully visible.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettingsPage,
  WriteAnswer,
  SettingOption,
  ResolvedOptions,
  ValueSource,
} from "$lib/appSettings";

/// The loaded page for the open app, or null before the read settles.
export const appPage = writable<AppSettingsPage | null>(null);
/// True while the page is the FIXTURE (no broker under vite).
export const appPageMocked = writable(false);
/// Per-key broker refusals, shown quietly at the row that caused them.
export const writeErrors = writable<Record<string, string>>({});

// i18n-foreign: a third-party app's own setting labels. They arrive from the
// broker as data and are rendered verbatim, because this page cannot hold a
// catalog for an arbitrary app's words. The fixture is not ours for the same
// reason the real schema is not.
const FIXTURE: AppSettingsPage = {
  appId: "com.example.editor",
  schema: {
    version: 3,
    sections: [
      {
        label: "Editing",
        description: "How the editor behaves while you type.",
        order: 1,
        items: [
          { key: "editor.autosave", type: "bool", label: "Save automatically", description: "Write changes to disk as you type", default: true },
          {
            key: "editor.autosave_delay",
            type: "duration",
            label: "Save after",
            unit: "seconds",
            min: 1,
            max: 120,
            default: 5,
            visible_when: { key: "editor.autosave", equals: "true" },
          },
          { key: "editor.tab_width", type: "int", label: "Tab width", unit: "spaces", min: 1, max: 16, default: 4 },
          {
            key: "editor.line_endings",
            type: "enum",
            label: "Line endings",
            options: [
              { value: "unix", label: "Unix", description: "LF, the norm everywhere but Windows" },
              { value: "windows", label: "Windows", description: "CRLF, for files shared with Windows tools" },
              { value: "keep", label: "Keep as found", description: "Whatever the file already uses" },
            ],
            default: "keep",
          },
          { key: "editor.font_size", type: "float", label: "Font size", unit: "pt", min: 8, max: 32, default: 11.5 },
        ],
      },
      {
        label: "Files",
        order: 2,
        items: [
          { key: "files.ignore", type: "string_list", label: "Ignored patterns", description: "Files the search and the sidebar skip", default: ["*.tmp", "node_modules"] },
          { key: "files.backup_dir", type: "path", label: "Backup folder", description: "Where timed backups land" },
          { key: "sync.token", type: "secret_ref", label: "Sync token", description: "The credential for the optional sync service" },
        ],
      },
      {
        label: "Appearance",
        order: 3,
        items: [
          { key: "ui.accent", type: "color", label: "Accent colour", default: "#6aa9e0" },
          { key: "ui.theme", type: "enum", label: "Editor theme", options: [], options_from: "installed_themes" },
          { key: "keys.command_palette", type: "keybind", label: "Command palette", default: "Ctrl+Shift+P" },
        ],
      },
      {
        label: "Advanced",
        order: 4,
        items: [
          {
            key: "engine.parser",
            type: "enum",
            label: "Parser engine",
            tags: ["advanced"],
            options: [
              { value: "incremental", label: "Incremental", description: "Fast reparse of only what changed" },
              { value: "full", label: "Full", description: "Reparse the whole file each edit; slower, simpler" },
            ],
            default: "incremental",
          },
          {
            key: "engine.flags",
            type: "raw",
            label: "Engine flags",
            tags: ["advanced"],
            description: "Irregular tuning values this page cannot express as controls",
          },
          {
            key: "legacy.telemetry",
            type: "bool",
            label: "Legacy usage pings",
            tags: ["advanced"],
            deprecated_message: "Goes away in version 4; the app no longer sends anything.",
            default: false,
          },
          { key: "windows.session", type: "handoff", label: "Session windows", description: "Arranged in the app's own window", handoff: { window: "session-manager" } },
        ],
      },
    ],
  },
  values: {
    "editor.autosave": true,
    "editor.autosave_delay": 12,
    "editor.tab_width": 4,
    "editor.line_endings": "keep",
    "editor.font_size": 11.5,
    "files.ignore": ["*.tmp", "node_modules"],
    "files.backup_dir": "~/Backups/editor",
    "sync.token": "vault:sync-token-2",
    "ui.accent": "#6aa9e0",
    "ui.theme": "",
    "keys.command_palette": "Ctrl+Shift+P",
    "engine.parser": "incremental",
    "engine.flags": 'threads = 4\nheap = "512M"',
    "legacy.telemetry": false,
    "printing.duplex": true,
  },
  userSet: ["editor.autosave_delay", "files.backup_dir", "sync.token"],
  unavailable: {
    "ui.theme": "The theme registry isn't reachable right now.",
  },
};

/// Load one app's settings page. Live: `app_settings_page` (null when the app
/// declares no schema - the section is then honestly absent); fixture under vite
/// for the demo app only, so other apps show the honest no-schema state.
export async function loadAppSettings(appId: string): Promise<void> {
  appPage.set(null);
  writeErrors.set({});
  try {
    const page = await invoke<AppSettingsPage | null>("app_settings_page", { appId });
    appPage.set(page);
    appPageMocked.set(false);
  } catch {
    // Only under vite, and only for the demo id. A real session that could not
    // read the page shows no page: an invented settings page is one the user
    // then edits, and those writes go to a schema nobody confirmed.
    const demo = import.meta.env.DEV && appId === FIXTURE.appId;
    appPage.set(demo ? structuredClone(FIXTURE) : null);
    appPageMocked.set(demo);
  }
}

function applyLocal(key: string, value: unknown, markUserSet: boolean): void {
  appPage.update((p) => {
    if (!p) return p;
    const userSet = markUserSet && !p.userSet.includes(key) ? [...p.userSet, key] : p.userSet;
    return { ...p, values: { ...p.values, [key]: value }, userSet };
  });
}

/// Write one key through the broker. The broker validates (schema, scope,
/// bounds); a refusal shows its message at the row, and the local view snaps
/// back to what the broker last served.
export async function writeKey(key: string, value: unknown): Promise<void> {
  const before = getValue(key);
  applyLocal(key, value, true);
  writeErrors.update((e) => {
    const next = { ...e };
    delete next[key];
    return next;
  });
  try {
    const answer = await invoke<WriteAnswer>("app_settings_write", {
      appId: currentAppId(),
      writes: [{ key, value }],
    });
    if (!answer.ok) {
      applyLocal(key, before, false);
      writeErrors.update((e) => ({ ...e, [answer.refusedKey || key]: answer.message }));
    }
  } catch {
    // Broker unwired under vite: the optimistic write stands on the fixture.
  }
}

/// Reset a key to its shipped default (a write of the declared default; the
/// broker clears the user layer).
export async function resetKey(key: string, defaultValue: unknown): Promise<void> {
  await writeKey(key, defaultValue);
  appPage.update((p) => (p ? { ...p, userSet: p.userSet.filter((k) => k !== key) } : p));
}

/// The raw-TOML escape hatch for a single declared key.
export async function writeRaw(key: string, text: string): Promise<string | null> {
  try {
    const answer = await invoke<WriteAnswer>("app_settings_write_raw", {
      appId: currentAppId(),
      key,
      text,
    });
    if (!answer.ok) return answer.message;
    applyLocal(key, text, true);
    return null;
  } catch {
    applyLocal(key, text, true);
    return null;
  }
}

/// Resolve a dynamic option source (`options_from`) through the broker.
/// Under vite a small fixture list stands in per source so the resolved-enum
/// path stays visible; a source with no fixture rejects, and the widget shows
/// the honest could-not-ask line.
export async function resolveOptions(source: ValueSource): Promise<SettingOption[]> {
  try {
    // The command returns an envelope, not a bare array. Annotating it as
    // `SettingOption[]` handed the widget the envelope object, and its `.map`
    // over the "list" threw at render - past the catch below, so the honest
    // could-not-ask line never appeared either.
    const res = await invoke<ResolvedOptions>("settings_resolve_options", { source });
    if (!res.available) {
      throw new Error(res.reason || "unresolved");
    }
    return res.options;
  } catch {
    const fixture = RESOLVE_FIXTURE[source];
    if (!fixture) throw new Error("unresolved");
    return fixture;
  }
}

// i18n-foreign: what `settings_resolve_options` returns - device names from
// PipeWire, locales from the system database. Supplier text, rendered as it
// comes. The locale rows are the arguable ones: a tag plus Intl.DisplayNames
// would render them in the reader's language, and that is a change to what
// the broker returns rather than a string to translate here.
const RESOLVE_FIXTURE: Partial<Record<ValueSource, SettingOption[]>> = {
  audio_outputs: [
    { value: "alsa_output.pci-0000_00_1f.3", label: "Built-in speakers", description: "The laptop's own output" },
    { value: "bluez_output.headphones", label: "Headphones", description: "Connected over Bluetooth" },
  ],
  locales: [
    { value: "en_US.UTF-8", label: "English (United States)", description: "The system default" },
    { value: "de_AT.UTF-8", label: "Deutsch (Österreich)", description: "German with Austrian formats" },
  ],
};

let _appId: string | null = null;
appPage.subscribe((p) => {
  if (p) _appId = p.appId;
});
function currentAppId(): string | null {
  return _appId;
}
function getValue(key: string): unknown {
  let v: unknown;
  appPage.subscribe((p) => (v = p?.values[key]))();
  return v;
}

/// Identity and provenance metadata for one app: version, publisher and the
/// store entry. Live: `settings_app_meta` (a seam - not in src-tauri yet);
/// under vite the fixture stands in for the demo app only.
export interface AppMeta {
  version: string | null;
  publisher: string | null;
  /// The AppStream component id of the store entry, null when the app did not
  /// come from the store.
  storeComponent: string | null;
}

/// The open app's metadata, or null while unknown.
export const appMeta = writable<AppMeta | null>(null);
/// True while the metadata is the FIXTURE.
export const appMetaMocked = writable(false);

const META_FIXTURE: AppMeta = {
  version: "3.2.1",
  publisher: "Example Software",
  storeComponent: "com.example.editor",
};

/// Load one app's metadata; absent command or unknown app resolves to null so
/// the page simply omits what it cannot state.
export async function loadAppMeta(appId: string): Promise<void> {
  appMeta.set(null);
  try {
    appMeta.set(await invoke<AppMeta | null>("settings_app_meta", { appId }));
    appMetaMocked.set(false);
  } catch {
    const demo = import.meta.env.DEV && appId === FIXTURE.appId;
    appMeta.set(demo ? { ...META_FIXTURE } : null);
    appMetaMocked.set(demo);
  }
}

/// The general facts about one app (per-app-settings-plan.md §4.6): what it
/// opens, its storage footprint and the default-app roles it holds. Live:
/// `settings_app_general` + `settings_app_clear_cache` (seams); fixture under
/// vite for the demo app.
export interface AppGeneral {
  /// File types and link kinds the app registered handlers for.
  opens: string[];
  /// App data size in bytes, null when not yet measured.
  appBytes: number | null;
  /// Cache size in bytes, null when not yet measured.
  cacheBytes: number | null;
  /// Roles this app is the system default for.
  defaultFor: string[];
}

/// The open app's general facts, or null while unknown.
export const appGeneral = writable<AppGeneral | null>(null);

const GENERAL_FIXTURE: AppGeneral = {
  opens: ["Markdown", "Plain text", "Web links"],
  appBytes: 38_000_000,
  cacheBytes: 112_000_000,
  defaultFor: ["Text files"],
};

/// Load one app's general facts; absent command or unknown app resolves to
/// null and the section stays away.
export async function loadAppGeneral(appId: string): Promise<void> {
  appGeneral.set(null);
  try {
    appGeneral.set(await invoke<AppGeneral | null>("settings_app_general", { appId }));
  } catch {
    // No mocked flag on this one, so the fixture would be unlabelled if it ever
    // reached a real session. It cannot now.
    appGeneral.set(import.meta.env.DEV && appId === FIXTURE.appId ? structuredClone(GENERAL_FIXTURE) : null);
  }
}

/// Clear the app's cache. Idempotent: clearing an already-empty cache is a
/// no-op, so the button never needs a confirm.
export async function clearAppCache(appId: string): Promise<void> {
  appGeneral.update((g) => (g ? { ...g, cacheBytes: 0 } : g));
  try {
    await invoke("settings_app_clear_cache", { appId });
  } catch {
    // Seam unwired: the local zero stands for the session.
  }
}
