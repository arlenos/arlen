/// Picker-specific UI state the kit browser controller does not own:
/// the caller's active type-filter, the Save filename, and a transient
/// notice. The controller owns directory / entries / selection / sort /
/// view-mode / hidden; this store is only the portal-chrome state.
///
/// Module-level `$state` so the chrome (the filter PopoverSelect, the
/// SaveBar) and the main view share one source without prop drilling.

import type { FileEntry } from "@arlen/ui-kit/components/browser";
import type { FileFilter } from "$lib/types/protocol";

interface PickerUiState {
  activeFilter: FileFilter | null;
  saveFilename: string;
  /// Transient user-visible notice (e.g. the multi-select cap); auto-
  /// clears after 3 s, null when nothing to show.
  notice: string | null;
}

const state = $state<PickerUiState>({
  activeFilter: null,
  saveFilename: "",
  notice: null,
});

export function getUiState(): PickerUiState {
  return state;
}

export function setActiveFilter(filter: FileFilter | null) {
  state.activeFilter = filter;
}

export function setSaveFilename(name: string) {
  state.saveFilename = name;
}

let noticeTimer: ReturnType<typeof setTimeout> | null = null;

/// Show a transient notice for 3 s, replacing any current one (the
/// picker window is small; stacking would overflow). Used for the
/// multi-select cap announcement.
export function showNotice(message: string) {
  if (noticeTimer !== null) clearTimeout(noticeTimer);
  state.notice = message;
  noticeTimer = setTimeout(() => {
    state.notice = null;
    noticeTimer = null;
  }, 3000);
}

/// Multi-select size cap. The D-Bus message-size limit is ~16 KB; with
/// long paths a thousand-file selection overflows, so the UI caps at
/// 256 and surfaces a notice at the limit.
export const MULTI_SELECT_CAP = 256;

/// Validate a Save filename. The picker builds save paths as
/// `<currentDir>/<filename>`; a name with separators or `..` could
/// escape the displayed directory and hand a sandboxed caller a
/// writable export elsewhere. The daemon's `validate_save_path` is the
/// second line of defence. Returns null when acceptable, else an error
/// string for inline display.
export function validateFilename(name: string): string | null {
  if (!name || name.length === 0) return "Filename is required.";
  if (name === "." || name === "..") return "Reserved name.";
  if (name.includes("/")) return "Slashes are not allowed in the filename.";
  if (name.includes("\0")) return "Filename cannot contain a NUL byte.";
  for (const c of name) {
    if (c.charCodeAt(0) < 0x20) return "Filename cannot contain control characters.";
  }
  return null;
}

/// Match an entry name against a single glob pattern. Simple ends-with
/// for `*.ext` covers the realistic portal pattern set; full glob would
/// pull a dependency for no real benefit.
function matchesGlob(name: string, pattern: string): boolean {
  if (pattern === "*") return true;
  if (pattern.startsWith("*.")) {
    return name.toLowerCase().endsWith(pattern.slice(1).toLowerCase());
  }
  return name.toLowerCase() === pattern.toLowerCase();
}

/// Best-effort MIME match from extension. Real MIME detection needs
/// xdg-mime; the common image/audio/video cases map cleanly already.
function matchesMime(name: string, mimeType: string): boolean {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  const map: Record<string, string[]> = {
    "image/png": ["png"],
    "image/jpeg": ["jpg", "jpeg"],
    "image/gif": ["gif"],
    "image/webp": ["webp"],
    "image/svg+xml": ["svg"],
    "image/heic": ["heic", "heif"],
    "application/pdf": ["pdf"],
    "text/plain": ["txt", "log", "md"],
    "text/markdown": ["md"],
    "audio/mpeg": ["mp3"],
    "audio/ogg": ["ogg"],
    "video/mp4": ["mp4"],
    "video/webm": ["webm"],
  };
  if (mimeType.endsWith("/*")) {
    const prefix = mimeType.slice(0, -2);
    return Object.entries(map).some(
      ([type, exts]) => type.startsWith(`${prefix}/`) && exts.includes(ext),
    );
  }
  return (map[mimeType] ?? []).includes(ext);
}

/// The host filter the kit FileBrowser applies per row. Directories
/// always pass so the user can navigate while a file filter is active;
/// files pass when no filter is set or any of the filter's patterns
/// matches. Mirrors the FM's `filter` prop convention.
export function filterPredicate(
  filter: FileFilter | null,
): (entry: FileEntry) => boolean {
  if (!filter) return () => true;
  return (entry: FileEntry) => {
    if (entry.kind === "directory") return true;
    for (const pat of filter.patterns) {
      if (pat.kind === "glob" && matchesGlob(entry.name, pat.pattern)) return true;
      if (pat.kind === "mime" && matchesMime(entry.name, pat.mimeType)) return true;
    }
    return false;
  };
}
