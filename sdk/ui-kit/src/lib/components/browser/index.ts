/// The Browser archetype primitives (design-system.md §5.3): the
/// shared file-browser family, hosted by the FM app and the confined
/// xdg picker (and composed à la carte by the pickers). The
/// controller is the seam: one instance per location/tab, adapter
/// supplied by the host.

export { default as FileBrowser } from "./FileBrowser.svelte";
export { default as FileList } from "./FileList.svelte";
export { default as FileGrid } from "./FileGrid.svelte";
export { default as FileTile } from "./FileTile.svelte";
export { default as MillerColumns } from "./MillerColumns.svelte";
export { default as FileRow } from "./FileRow.svelte";
export { default as Breadcrumb } from "./Breadcrumb.svelte";
export { default as PlacesSidebar } from "./PlacesSidebar.svelte";
export { createBrowserState, type BrowserState, type ViewMode } from "./controller";
export { breadcrumb, isVirtualLocation, locationCrumbs } from "./breadcrumb";
export { Selection } from "./selection";
export { entryIcon, placeIcon } from "./icons";
export { formatModified, formatSize } from "./format";
export {
  joinPath,
  parentPath,
  type BrowserAdapter,
  type Crumb,
  type EntryKind,
  type FileEntry,
  type Place,
  type PlaceGroup,
  type SortKey,
  type SortSpec,
} from "./types";
