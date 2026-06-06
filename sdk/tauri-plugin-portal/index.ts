/**
 * @lunaris/tauri-plugin-portal
 *
 * TypeScript bindings for the Lunaris portal plugin. Wraps the
 * standard `org.freedesktop.portal.Desktop` FileChooser and
 * OpenURI interfaces — under a Lunaris session the calls are
 * served by `xdg-desktop-portal-lunaris`; under GNOME/KDE the
 * frontend daemon falls through to whichever backend is
 * configured for that desktop.
 *
 * # User cancellation vs. error
 *
 * `pickFile`, `pickDirectory`, `saveFile`, `saveFiles` return
 * `null` when the user dismisses the dialog. Errors throw — they
 * are reserved for actual failures (portal unavailable, scheme
 * rejected, backend failure).
 *
 * # Example
 *
 * ```typescript
 * import { pickDirectory } from '@lunaris/tauri-plugin-portal';
 *
 * const dir = await pickDirectory({ title: 'Choose folder' });
 * if (dir !== null) {
 *   console.log('user picked:', dir);
 * }
 * ```
 */

import { invoke } from "@tauri-apps/api/core";

// ── Types ────────────────────────────────────────────────────────

export type FilterPattern =
  | { kind: "glob"; pattern: string }
  | { kind: "mime"; mimeType: string };

export interface FileFilter {
  name: string;
  patterns: FilterPattern[];
}

export interface PickFileOptions {
  /** Window title shown in the picker. */
  title?: string;
  /** Allow multi-select. Default false. */
  multiple?: boolean;
  /** Modal hint. Wayland has no cross-app modal concept. */
  modal?: boolean;
  /** Pre-populate filters. First filter is selected by default. */
  filters?: FileFilter[];
  /** Pre-select a specific filter from `filters`. */
  currentFilter?: FileFilter;
  /** Initial directory; falls back to $HOME if unset or invalid. */
  currentFolder?: string;
}

export interface SaveFileOptions {
  title?: string;
  modal?: boolean;
  filters?: FileFilter[];
  currentFilter?: FileFilter;
  /** Suggested filename to pre-fill. */
  currentName?: string;
  currentFolder?: string;
  /** Pre-select a specific existing file (for "Save As"). */
  currentFile?: string;
}

export interface SaveFilesOptions {
  title?: string;
  modal?: boolean;
  /** Files to save. Picker chooses the directory; on confirm
   *  each filename is appended. */
  files?: string[];
  currentFolder?: string;
}

export interface OpenUriOptions {
  /** Whether the user should be prompted before opening. */
  ask?: boolean;
  /** Whether the URI should be opened with write permission.
   *  Only meaningful for `file://` URIs. */
  writable?: boolean;
}

interface PickerResultPicked {
  type: "picked";
  uris: string[];
}
interface PickerResultCancelled {
  type: "cancelled";
}
type PickerResult = PickerResultPicked | PickerResultCancelled;

// ── Internal helpers ────────────────────────────────────────────

function pickedFirstUri(result: PickerResult): string | null {
  if (result.type === "cancelled") return null;
  return result.uris[0] ?? null;
}

function pickedAllUris(result: PickerResult): string[] | null {
  if (result.type === "cancelled") return null;
  return result.uris;
}

function defaultedDirectoryFlag(opts: PickFileOptions): PickFileOptions {
  // Public callers never set `directory` themselves; the Rust
  // pickDirectory entry sets it internally. We keep the option
  // out of the TS PickFileOptions type to avoid confusion and
  // strip it here defensively if a caller ever tries.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const stripped: any = { ...opts };
  delete stripped.directory;
  return stripped as PickFileOptions;
}

// ── FileChooser ─────────────────────────────────────────────────

/**
 * Pick one or more existing files. Returns the selected URIs as
 * an array, or `null` if the user cancelled.
 */
export async function pickFile(
  options: PickFileOptions = {}
): Promise<string[] | null> {
  const result = await invoke<PickerResult>("plugin:lunaris-portal|pick_file", {
    options: defaultedDirectoryFlag(options),
  });
  return pickedAllUris(result);
}

/**
 * Pick a single directory. Returns the URI, or `null` if cancelled.
 */
export async function pickDirectory(
  options: PickFileOptions = {}
): Promise<string | null> {
  const result = await invoke<PickerResult>(
    "plugin:lunaris-portal|pick_directory",
    {
      options: defaultedDirectoryFlag(options),
    }
  );
  return pickedFirstUri(result);
}

/**
 * Save a single file. Returns the URI, or `null` if cancelled.
 */
export async function saveFile(
  options: SaveFileOptions = {}
): Promise<string | null> {
  const result = await invoke<PickerResult>("plugin:lunaris-portal|save_file", {
    options,
  });
  return pickedFirstUri(result);
}

/**
 * Save multiple files into a single directory. Returns one URI
 * per file, or `null` if cancelled.
 */
export async function saveFiles(
  options: SaveFilesOptions = {}
): Promise<string[] | null> {
  const result = await invoke<PickerResult>(
    "plugin:lunaris-portal|save_files",
    {
      options,
    }
  );
  return pickedAllUris(result);
}

// ── OpenURI ─────────────────────────────────────────────────────

/**
 * Open a URI in the user's preferred handler. http(s)/mailto/tel
 * pass through to xdg-open; file:// is sandbox-validated for
 * confined callers; everything else is rejected.
 *
 * Throws on rejection or backend failure.
 */
export async function openUri(
  uri: string,
  options: OpenUriOptions = {}
): Promise<void> {
  await invoke("plugin:lunaris-portal|open_uri", { uri, options });
}
