/// The kit-owned icon resolution for the browser archetype: pure TS,
/// zero IPC, so the confined xdg picker gets the same icons without
/// the FM app. Extension to category to Lucide component; the themed
/// icon set (seti-derived) and the KG-state overlays replace this
/// through the `icon` snippet seam on FileRow/FileTile later.
import type { Icon } from "@lucide/svelte";
import {
  Archive,
  Cloud,
  Download,
  File,
  FileCode,
  FileText,
  Folder,
  FolderDot,
  FolderSymlink,
  HardDrive,
  House,
  Image,
  Monitor,
  Music,
  Search,
  Trash2,
  Usb,
  Video,
} from "@lucide/svelte";
import type { EntryKind, FileEntry } from "./types";

const BY_EXT: Record<string, typeof Icon> = {};
const add = (icon: typeof Icon, exts: string[]) => {
  for (const e of exts) BY_EXT[e] = icon;
};
add(FileText, ["md", "txt", "pdf", "rtf", "odt", "doc", "docx", "tex", "bib"]);
add(Image, ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "avif", "tiff"]);
add(Music, ["mp3", "flac", "ogg", "wav", "opus", "m4a", "aac"]);
add(Video, ["mp4", "mkv", "webm", "avi", "mov", "m4v"]);
add(Archive, ["zip", "tar", "gz", "xz", "zst", "bz2", "7z", "rar", "iso", "deb", "rpm"]);
add(FileCode, [
  "rs", "ts", "js", "svelte", "py", "c", "h", "cpp", "go", "java", "sh",
  "zsh", "css", "html", "json", "toml", "yml", "yaml", "xml", "sql", "lock",
]);

function ext(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

/// The icon for a listing entry.
export function entryIcon(entry: { name: string; kind: EntryKind } | FileEntry): typeof Icon {
  if (entry.kind === "directory") return Folder;
  if (entry.kind === "symlink") return FolderSymlink;
  return BY_EXT[ext(entry.name)] ?? File;
}

/// The icon for a sidebar place, by its host-provided icon key.
const PLACE_ICONS: Record<string, typeof Icon> = {
  home: House,
  documents: FileText,
  downloads: Download,
  pictures: Image,
  music: Music,
  videos: Video,
  desktop: Monitor,
  system: HardDrive,
  usb: Usb,
  cloud: Cloud,
  trash: Trash2,
  project: FolderDot,
  search: Search,
};

export function placeIcon(key: string): typeof Icon {
  return PLACE_ICONS[key] ?? Folder;
}
