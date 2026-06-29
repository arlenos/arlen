<script lang="ts">
  /// The picker main view, hosted on the shared `@arlen/ui-kit` browser
  /// family so it reads as one design language with the file manager:
  /// thumbnails, list/grid, a places sidebar, search, sort - the FM's
  /// browser, specialized for the portal. The portal-specific chrome
  /// stays: the open / save / folder modes, the caller's type-filter,
  /// the SaveBar, multi-select confirm, and the trust cue (the powerbox
  /// framing - this dialog grants the requesting app access to exactly
  /// the selection, nothing else).
  ///
  /// The confinement is the controller's `root` bound plus
  /// `allowVirtual: false`: the picker only ever lists real folders
  /// through the daemon's cap-std FS, never a KG virtual location. The
  /// security scoping (return only the selection) is the daemon's; the
  /// view never assumes ambient filesystem access.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { ArrowUp, Eye, EyeOff, LayoutGrid, List, Search, ShieldCheck } from "@lucide/svelte";
  import {
    Breadcrumb,
    FileBrowser,
    PlacesSidebar,
    createBrowserState,
    joinPath,
    type BrowserState,
    type FileEntry,
    type Place,
    type PlaceGroup,
    type ViewMode,
  } from "@arlen/ui-kit/components/browser";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import {
    PopoverSelect,
    type PopoverSelectOption,
  } from "@arlen/ui-kit/components/ui/popover-select";

  import { applyTheme, initPickerBridge, respond } from "$lib/ipc";
  import { pickerAdapter } from "$lib/adapter";
  import { getPickState } from "$lib/stores/pickState.svelte";
  import {
    MULTI_SELECT_CAP,
    filterPredicate,
    getUiState,
    setActiveFilter,
    setSaveFilename,
    showNotice,
    validateFilename,
  } from "$lib/stores/pickerUi.svelte";
  import { conventionalPlaces, recentGroup, resolveHome } from "$lib/places";
  import type { FileFilter, PickerRequest } from "$lib/types/protocol";
  import SaveBar from "$lib/components/SaveBar.svelte";

  const pickState = getPickState();
  const ui = getUiState();

  // One controller per request; rebuilt when a new request handle
  // arrives so a fresh dialog never inherits the prior navigation.
  let controller = $state<BrowserState | null>(null);
  let lastInitedHandle = $state<string | null>(null);

  let home = $state("/home");
  let placeGroups = $state<PlaceGroup[]>([]);
  let selected = $state<FileEntry[]>([]);
  let searchText = $state("");

  // Mirrors of the controller's reactive state for the chrome (the
  // header path, the view toggle, the hidden toggle).
  let currentDir = $state("");
  let viewMode = $state<ViewMode>("list");
  let showHidden = $state(false);

  $effect(() => {
    const c = controller;
    if (!c) return;
    const unsubs = [
      c.path.subscribe((v) => (currentDir = v)),
      c.viewMode.subscribe((v) => (viewMode = v)),
      c.showHidden.subscribe((v) => (showHidden = v)),
    ];
    return () => unsubs.forEach((u) => u());
  });

  onMount(() => {
    applyTheme();
    initPickerBridge();
  });

  // On a new request: resolve the start dir + home, build the places,
  // seed the Save filename + the active filter, and create the
  // controller. Honours the caller's currentName / currentFile /
  // currentFilter precedence (the original picker's rules).
  $effect(() => {
    const req = pickState.request;
    if (!req || req.handle === lastInitedHandle) return;
    lastInitedHandle = req.handle;
    void initForRequest(req);
  });

  async function initForRequest(req: PickerRequest) {
    searchText = "";
    selected = [];

    if (req.type === "saveFile") {
      if (req.currentName) setSaveFilename(req.currentName);
      else if (req.currentFile) setSaveFilename(basename(req.currentFile));
      else setSaveFilename("");
    } else {
      setSaveFilename("");
    }

    if ("currentFilter" in req && req.currentFilter) setActiveFilter(req.currentFilter);
    else setActiveFilter(null);

    const provided =
      ("currentFolder" in req && req.currentFolder) ||
      ("currentFile" in req && req.currentFile ? parentDir(req.currentFile) : null);
    const start = await invoke<string>("resolve_start_dir", { provided }).catch(
      () => provided ?? "/home",
    );
    home = await resolveHome();

    controller = createBrowserState(pickerAdapter, {
      initial: start,
      root: "/",
      allowVirtual: false,
    });

    const groups: PlaceGroup[] = [conventionalPlaces(home)];
    const recent = await recentGroup();
    if (recent) groups.push(recent);
    placeGroups = groups;
  }

  function basename(path: string): string {
    const i = path.lastIndexOf("/");
    return i >= 0 ? path.slice(i + 1) : path;
  }
  function parentDir(path: string): string {
    const i = path.lastIndexOf("/");
    return i > 0 ? path.slice(0, i) : "/";
  }

  // ---- Mode predicates -------------------------------------------------

  function isOpenFile(r: PickerRequest): r is Extract<PickerRequest, { type: "openFile" }> {
    return r.type === "openFile";
  }
  function isSaveFile(r: PickerRequest): r is Extract<PickerRequest, { type: "saveFile" }> {
    return r.type === "saveFile";
  }
  function isSaveFiles(r: PickerRequest): r is Extract<PickerRequest, { type: "saveFiles" }> {
    return r.type === "saveFiles";
  }

  const multiple = $derived.by(() => {
    const r = pickState.request;
    return r && isOpenFile(r) ? r.multiple : false;
  });
  const directoriesOnly = $derived.by(() => {
    const r = pickState.request;
    return r && isOpenFile(r) ? r.directory : false;
  });
  const filters = $derived.by(() => {
    const r = pickState.request;
    if (!r) return [];
    if (isOpenFile(r) || isSaveFile(r)) return r.filters;
    return [];
  });

  // The view switcher + the type-filter both ride kit primitives. The
  // segmented control's value is a plain string mirrored from the
  // controller; the filter is a PopoverSelect over the caller's
  // filters plus the always-present "All files" escape hatch.
  const VIEW_OPTIONS: { value: string; label: string; icon: typeof List }[] = [
    { value: "list", label: "List view", icon: List },
    { value: "grid", label: "Grid view", icon: LayoutGrid },
  ];
  const ALL_FILTER = "__all__";
  const filterOptions = $derived<PopoverSelectOption[]>([
    ...filters.map((f) => ({ value: f.name, label: f.name })),
    { value: ALL_FILTER, label: "All files" },
  ]);
  const filterValue = $derived(ui.activeFilter?.name ?? ALL_FILTER);

  function onFilterChange(value: string) {
    if (value === ALL_FILTER) {
      setActiveFilter(null);
      return;
    }
    setActiveFilter(filters.find((f: FileFilter) => f.name === value) ?? null);
  }

  // The kit `filter` prop: the caller's type-filter AND the local search
  // box. Directories always pass the type-filter so navigation works;
  // the search box narrows everything by substring.
  const rowFilter = $derived.by(() => {
    const typePred = filterPredicate(ui.activeFilter);
    const q = searchText.trim().toLowerCase();
    return (entry: FileEntry) =>
      typePred(entry) && (q === "" || entry.name.toLowerCase().includes(q));
  });

  const title = $derived.by(() => {
    const r = pickState.request;
    if (!r) return "Open file";
    if ("title" in r && r.title) return r.title;
    if (isSaveFile(r) || isSaveFiles(r)) return "Save file";
    if (directoriesOnly) return "Choose folder";
    return "Open file";
  });

  const confirmLabel = $derived.by(() => {
    const r = pickState.request;
    if (!r) return "Open";
    if (isSaveFile(r) || isSaveFiles(r)) return "Save";
    if (directoriesOnly) return "Choose folder";
    return "Open";
  });

  const confirmDisabled = $derived.by(() => {
    const r = pickState.request;
    if (!r || pickState.busy) return true;
    if (isOpenFile(r)) {
      if (directoriesOnly && !multiple) return false; // current dir is the answer
      return selected.length === 0;
    }
    if (isSaveFile(r)) return validateFilename(ui.saveFilename) !== null;
    if (isSaveFiles(r)) return r.files.length === 0;
    return true;
  });

  // The trust cue: name the requesting app and the exact grant. Honest
  // about what the confirm hands over (a file to open, a save location,
  // a folder) - never cosmetic.
  const appLabel = $derived.by(() => {
    const r = pickState.request;
    const id = r && "appId" in r ? r.appId : "";
    if (!id) return "The requesting app";
    const seg = id.split(".").pop() ?? id;
    return seg.charAt(0).toUpperCase() + seg.slice(1);
  });
  const trustLine = $derived.by(() => {
    const r = pickState.request;
    if (!r) return "";
    if (isSaveFile(r) || isSaveFiles(r))
      return `${appLabel} gets to save to the location you choose`;
    if (directoriesOnly) return `${appLabel} gets access to the folder you choose`;
    return `${appLabel} gets access to ${multiple ? "the files" : "the file"} you choose`;
  });

  // ---- Actions ---------------------------------------------------------

  function pathsFor(entries: FileEntry[]): string[] {
    return entries.map((e) => joinPath(currentDir, e.name));
  }

  async function confirm() {
    const r = pickState.request;
    if (!r) return;

    if (isOpenFile(r)) {
      if (directoriesOnly) {
        const paths =
          multiple && selected.length > 0 ? pathsFor(selected) : [currentDir];
        await respond({ type: "picked", handle: r.handle, paths, currentFilter: ui.activeFilter });
        return;
      }
      if (selected.length === 0) return;
      let paths = pathsFor(selected);
      if (paths.length > MULTI_SELECT_CAP) {
        showNotice(`Selection limited to ${MULTI_SELECT_CAP} files.`);
        paths = paths.slice(0, MULTI_SELECT_CAP);
      }
      await respond({ type: "picked", handle: r.handle, paths, currentFilter: ui.activeFilter });
      return;
    }

    if (isSaveFile(r)) {
      const name = ui.saveFilename.trim();
      if (validateFilename(name) !== null) return;
      const path = `${currentDir.replace(/\/$/, "")}/${name}`;
      const exists = await invoke<boolean>("file_exists", { path }).catch(() => false);
      if (exists && !window.confirm(`Replace ${name}?`)) return;
      await respond({ type: "picked", handle: r.handle, paths: [path], currentFilter: ui.activeFilter });
      return;
    }

    if (isSaveFiles(r)) {
      const dir = currentDir.replace(/\/$/, "");
      const paths = r.files.map((p) => `${dir}/${basename(p)}`);
      await respond({ type: "picked", handle: r.handle, paths, currentFilter: null });
    }
  }

  async function cancel() {
    const r = pickState.request;
    if (!r) return;
    await respond({ type: "cancelled", handle: r.handle });
  }

  // A file activation (double-click / Enter on a non-directory).
  // Directories navigate inside FileBrowser. In open mode a file
  // activation IS the confirmation; in save mode it reuses the name.
  function onActivate(entry: FileEntry, path: string) {
    const r = pickState.request;
    if (!r || entry.kind === "directory") return;
    if (isSaveFile(r)) {
      setSaveFilename(entry.name);
      return;
    }
    if (isOpenFile(r) && !directoriesOnly) {
      void respond({ type: "picked", handle: r.handle, paths: [path], currentFilter: ui.activeFilter });
    }
  }

  function navigateTo(path: string) {
    void controller?.navigate(path);
  }
  function onPlace(place: Place) {
    void controller?.navigate(place.path);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") cancel();
    else if (e.key === "h" && e.ctrlKey) {
      e.preventDefault();
      void controller?.setShowHidden(!showHidden);
    } else if (e.key === "Enter" && !confirmDisabled) {
      // Enter in the search/save fields is handled by the field itself.
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag !== "INPUT") confirm();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<main class="picker">
  {#if pickState.request && controller}
    <header data-tauri-drag-region>
      <div class="title-row" data-tauri-drag-region>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Up one directory"
          title="Up one directory"
          onclick={() => controller?.up()}
        >
          <ArrowUp strokeWidth={1.75} />
        </Button>
        <h1 data-tauri-drag-region>{title}</h1>
        <SegmentedControl
          options={VIEW_OPTIONS}
          value={viewMode}
          onchange={(v) => controller?.viewMode.set(v as ViewMode)}
        />
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={showHidden ? "Hide hidden files" : "Show hidden files"}
          title="Toggle hidden files (Ctrl+H)"
          onclick={() => controller?.setShowHidden(!showHidden)}
        >
          {#if showHidden}
            <Eye strokeWidth={1.75} />
          {:else}
            <EyeOff strokeWidth={1.75} />
          {/if}
        </Button>
      </div>
      <div class="nav-row">
        <Breadcrumb path={currentDir} homePath={home} onnavigate={navigateTo} />
        <div class="search">
          <Search class="search-icon size-3.5" strokeWidth={1.75} />
          <Input
            class="search-input"
            placeholder="Filter"
            bind:value={searchText}
            autocomplete="off"
            spellcheck="false"
            aria-label="Filter the listing by name"
          />
        </div>
      </div>
    </header>

    <div class="body">
      <aside class="sidebar">
        <PlacesSidebar groups={placeGroups} activePath={currentDir} onnavigate={onPlace} />
      </aside>
      <section class="browse">
        <FileBrowser
          {controller}
          filter={rowFilter}
          onactivate={onActivate}
          onselection={(list) => (selected = list)}
        />
      </section>
    </div>

    {#if isSaveFile(pickState.request)}
      <SaveBar location={currentDir} />
    {/if}

    <footer>
      <div class="trust">
        <ShieldCheck class="size-4" strokeWidth={1.75} />
        <span>{trustLine}</span>
      </div>
      <div class="action-row">
        {#if filters.length > 0}
          <PopoverSelect
            value={filterValue}
            options={filterOptions}
            onchange={onFilterChange}
            placeholder="All files"
            width="max-content"
          />
        {:else}
          <span></span>
        {/if}
        <div class="actions">
          <Button variant="outline" onclick={cancel} disabled={pickState.busy}>Cancel</Button>
          <Button variant="default" onclick={confirm} disabled={confirmDisabled}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </footer>

    {#if ui.notice}
      <div class="notice" role="status">{ui.notice}</div>
    {/if}
  {:else}
    <div class="idle">
      <p>Waiting for a request.</p>
    </div>
  {/if}
</main>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app);
    color: var(--color-fg-app);
  }

  header {
    flex-shrink: 0;
    padding: 10px 14px 12px;
    border-bottom: 1px solid var(--color-border);
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .title-row h1 {
    flex: 1;
    margin: 0;
    font-size: 0.9375rem;
    font-weight: 600;
  }

  .nav-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .nav-row :global(nav) {
    flex: 1;
    min-width: 0;
  }

  /* The kit Input carries the field chrome; the wrapper just sizes it
     and floats the search glyph over its leading padding. */
  .search {
    position: relative;
    flex-shrink: 0;
    width: 168px;
  }
  .search :global(.search-icon) {
    position: absolute;
    left: 10px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--color-fg-muted);
    pointer-events: none;
  }
  .search :global(.search-input) {
    padding-left: 30px;
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .sidebar {
    flex-shrink: 0;
    width: 184px;
    padding: 8px;
    overflow-y: auto;
    border-right: 1px solid var(--color-border);
    background: var(--color-bg-shell);
  }
  .browse {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  footer {
    flex-shrink: 0;
    border-top: 1px solid var(--color-border);
    background: var(--color-bg-card);
  }
  .trust {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 16px;
    font-size: 0.8125rem;
    color: var(--color-fg-muted);
    border-bottom: 1px solid color-mix(in srgb, var(--color-border) 60%, transparent);
  }
  .trust :global(svg) {
    flex-shrink: 0;
    color: var(--color-fg-app);
  }

  .action-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 16px;
  }
  .actions {
    display: flex;
    gap: 8px;
  }

  .notice {
    position: absolute;
    bottom: 76px;
    left: 50%;
    transform: translateX(-50%);
    padding: 7px 14px;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-lg, 0 12px 32px rgba(0, 0, 0, 0.4));
    font-size: 0.8125rem;
    color: var(--color-fg-app);
  }

  .idle {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-fg-muted);
  }
</style>
