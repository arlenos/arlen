/// The headless browser controller: one instance is one navigable
/// location with its own history, sort and listing - a tab IS a
/// controller, the host keeps N of them and mounts one FileBrowser.
/// Adapter results land in writable stores (the project rule: IPC
/// continuations never mutate `$state`), guarded by a navigation
/// generation so a slow listing can never overwrite a newer one.

import { derived, get, writable, type Readable, type Writable } from "svelte/store";
import type { BrowserAdapter, FileEntry, SortKey, SortSpec } from "./types";
import { joinPath, parentPath } from "./types";
import { isVirtualLocation } from "./breadcrumb";

/// How the browser renders a listing. Dual-pane is deliberately NOT a
/// view mode - it is a host layout of two browsers.
export type ViewMode = "list" | "grid" | "miller";

export interface BrowserState {
  /// The current location (host-opaque path).
  readonly path: Writable<string>;
  /// The render mode of this location (a tab keeps its mode).
  readonly viewMode: Writable<ViewMode>;
  /// The current listing; empty while loading or on error.
  readonly entries: Readable<FileEntry[]>;
  readonly loading: Readable<boolean>;
  /// The host-thrown listing error (a lay-readable sentence), or null.
  readonly error: Readable<string | null>;
  readonly sortKey: Readable<SortKey>;
  readonly ascending: Readable<boolean>;
  readonly showHidden: Readable<boolean>;
  readonly canBack: Readable<boolean>;
  readonly canForward: Readable<boolean>;
  readonly canUp: Readable<boolean>;
  /// The confinement boundary; navigation never leaves it.
  readonly root: string;
  /// Resolved thumbnail URLs by key (null = the host has none); tiles
  /// subscribe and fall back to their icon for misses and nulls.
  readonly thumbnails: Readable<ReadonlyMap<string, string | null>>;
  /// Whether the adapter can produce thumbnails at all.
  readonly hasThumbnails: boolean;
  /// A monotonic select-all signal: the mounted view selects every
  /// entry each time this increments. The headless controller owns no
  /// selection model (selection is view state in the FileBrowser), so a
  /// host command - a topbar "Select all" - reaches the view through
  /// this channel, the same store-driven indirection the listing uses.
  readonly selectAllSignal: Readable<number>;

  navigate(path: string): Promise<void>;
  back(): Promise<void>;
  forward(): Promise<void>;
  up(): Promise<void>;
  refresh(): Promise<void>;
  setSort(key: SortKey, ascending?: boolean): Promise<void>;
  setShowHidden(show: boolean): Promise<void>;
  /// A cached listing for a path (Miller's ancestor columns); resolves
  /// from the cache or lists through the adapter.
  listCached(path: string): Promise<FileEntry[]>;
  /// Ask for the entry's thumbnail (deduped, LRU-cached); the result
  /// lands in `thumbnails`. Cheap to call repeatedly per render.
  requestThumbnail(entry: FileEntry): void;
  /// The `thumbnails` key for an entry at the current location.
  thumbnailKeyFor(entry: FileEntry): string;
  /// Ask the mounted view to select every entry at this location (the
  /// topbar "Select all"); bumps [`selectAllSignal`].
  selectAll(): void;
}

export function createBrowserState(
  adapter: BrowserAdapter,
  opts: { initial: string; root?: string; allowVirtual?: boolean },
): BrowserState {
  const root = opts.root ?? "/";
  // The FM (root "/") also navigates to virtual KG locations (recent / trash /
  // project: / search:) - opaque non-path keys the adapter routes to the
  // location seam. A confined host (the xdg picker) leaves this off so its root
  // bound still blocks every non-path target.
  const allowVirtual = opts.allowVirtual ?? false;

  const path = writable(opts.initial);
  const viewMode = writable<ViewMode>("list");
  const entriesAll = writable<FileEntry[]>([]);
  const loading = writable(false);
  const error = writable<string | null>(null);
  const sortKey = writable<SortKey>("name");
  const ascending = writable(true);
  const showHidden = writable(false);
  // A monotonic counter; `selectAll()` bumps it and the mounted view
  // applies a select-all on each increment (selection is view state, so
  // the controller only signals).
  const selectAllSignal = writable(0);

  const backStack: string[] = [];
  const forwardStack: string[] = [];
  const canBack = writable(false);
  const canForward = writable(false);

  // Listing cache keyed by path + sort; busted wholesale on refresh.
  let cache = new Map<string, FileEntry[]>();
  let generation = 0;

  // Thumbnail cache: full path + mtime keys (path-scoped, so two
  // directories sharing a name never collide; an edited file refetches
  // via its new mtime). Nulls are cached too - a broken image must not
  // re-invoke on every scroll pass. Insertion order doubles as the LRU
  // order, touched on request; navigation keeps the map (the keys stay
  // valid), refresh distrusts it like the listing cache.
  const THUMB_LRU_MAX = 300;
  const thumbnails = writable<ReadonlyMap<string, string | null>>(new Map());
  const thumbInflight = new Set<string>();

  const thumbnailKeyFor = (entry: FileEntry): string =>
    `${joinPath(get(path), entry.name)} $${entry.modified_unix ?? 0}`;

  function settleThumbnail(key: string, url: string | null): void {
    thumbInflight.delete(key);
    // The IPC continuation writes only into the writable (the project
    // rule); a fresh Map reference is what makes prop reactivity fire.
    thumbnails.update((m) => {
      const next = new Map(m);
      next.set(key, url);
      while (next.size > THUMB_LRU_MAX) {
        next.delete(next.keys().next().value as string);
      }
      return next;
    });
  }

  function requestThumbnail(entry: FileEntry): void {
    if (!adapter.thumbnail || entry.kind !== "file") return;
    const key = thumbnailKeyFor(entry);
    const cur = get(thumbnails);
    if (cur.has(key)) {
      // Touch for recency so a visible tile never gets evicted under
      // entries scrolled past long ago.
      const next = new Map(cur);
      const v = next.get(key) ?? null;
      next.delete(key);
      next.set(key, v);
      thumbnails.set(next);
      return;
    }
    if (thumbInflight.has(key)) return;
    thumbInflight.add(key);
    adapter.thumbnail(joinPath(get(path), entry.name), entry).then(
      (url) => settleThumbnail(key, url),
      () => settleThumbnail(key, null),
    );
  }

  const sortSpec = (): SortSpec => ({
    key: get(sortKey),
    foldersFirst: true,
    ascending: get(ascending),
  });
  const cacheKey = (p: string) => {
    const s = sortSpec();
    return `${p} $${s.key}:${s.ascending}`;
  };

  function syncNav() {
    canBack.set(backStack.length > 0);
    canForward.set(forwardStack.length > 0);
  }

  async function load(p: string): Promise<void> {
    const gen = ++generation;
    loading.set(true);
    try {
      const key = cacheKey(p);
      let list = cache.get(key);
      if (!list) {
        list = await adapter.list(p, sortSpec());
        cache.set(key, list);
      }
      if (gen !== generation) return;
      entriesAll.set(list);
      error.set(null);
    } catch (e) {
      if (gen !== generation) return;
      entriesAll.set([]);
      error.set(String(e));
    }
    if (gen === generation) loading.set(false);
  }

  async function navigate(p: string): Promise<void> {
    // The root bound stops path composition above the confinement
    // boundary; a scheme URI the host itself handed out (a cloud
    // place) is not path composition and passes.
    if (
      !p.startsWith(root) &&
      !/^[a-z][a-z0-9+.-]*:\/\//.test(p) &&
      !(allowVirtual && isVirtualLocation(p))
    )
      return;
    const current = get(path);
    if (p === current) return;
    backStack.push(current);
    forwardStack.length = 0;
    syncNav();
    path.set(p);
    await load(p);
  }

  async function back(): Promise<void> {
    const prev = backStack.pop();
    if (prev === undefined) return;
    forwardStack.push(get(path));
    syncNav();
    path.set(prev);
    await load(prev);
  }

  async function forward(): Promise<void> {
    const next = forwardStack.pop();
    if (next === undefined) return;
    backStack.push(get(path));
    syncNav();
    path.set(next);
    await load(next);
  }

  async function up(): Promise<void> {
    const parent = parentPath(get(path), root);
    if (parent !== null) await navigate(parent);
  }

  async function refresh(): Promise<void> {
    cache = new Map();
    thumbnails.set(new Map());
    await load(get(path));
  }

  async function setSort(key: SortKey, asc?: boolean): Promise<void> {
    const flip = asc ?? (get(sortKey) === key ? !get(ascending) : true);
    sortKey.set(key);
    ascending.set(flip);
    await load(get(path));
  }

  async function setShowHidden(show: boolean): Promise<void> {
    showHidden.set(show);
  }

  function selectAll(): void {
    selectAllSignal.update((n) => n + 1);
  }

  async function listCached(p: string): Promise<FileEntry[]> {
    const key = cacheKey(p);
    const hit = cache.get(key);
    if (hit) return hit;
    const list = await adapter.list(p, sortSpec());
    cache.set(key, list);
    return list;
  }

  const entries = derived([entriesAll, showHidden], ([$all, $hidden]) =>
    $hidden ? $all : $all.filter((e) => !e.is_hidden),
  );

  const canUp = derived(path, ($p) => parentPath($p, root) !== null);

  // The initial location loads immediately; consumers see loading=true
  // until the first listing lands.
  void load(opts.initial);

  return {
    path,
    viewMode,
    entries,
    loading,
    error,
    sortKey,
    ascending,
    showHidden,
    canBack,
    canForward,
    canUp,
    root,
    thumbnails,
    hasThumbnails: typeof adapter.thumbnail === "function",
    selectAllSignal,
    selectAll,
    navigate,
    back,
    forward,
    up,
    refresh,
    setSort,
    setShowHidden,
    listCached,
    requestThumbnail,
    thumbnailKeyFor,
  };
}
