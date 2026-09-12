<script lang="ts">
  import { t, locale } from "$lib/i18n/messages";
  import { clearSurface } from "$lib/surface-clear";
  import { writable } from "$lib/stores/svelteRe.js";
  import { invoke } from "@tauri-apps/api/core";
  import { waypointerVisible, initWaypointerListeners, closeWaypointer } from "$lib/stores/waypointer.js";
  import {
    fetchAllApps, searchApps, launchApp as launchAppCmd, evaluateInput, executeShellCommand,
    openUrl, webSearch, type AppEntry, type WaypointerResult as InlineEvalResult,
  } from "$lib/stores/waypointerActions.js";
  import {
    windowResults, updateWindowResults, clearWindowResults,
    activateWindow, type WindowInfo,
  } from "$lib/stores/waypointerWindows.js";
  import {
    processResults, updateProcessResults, clearProcessResults,
    killProcess, formatBytes, type ProcessInfo,
  } from "$lib/stores/waypointerProcesses.js";
  import {
    Command, CommandInput, CommandList,
    CommandGroup, CommandItem, CommandSeparator,
  } from "@arlen/ui-kit/components/ui/command/index.js";
  import { AppWindow, BookOpen, Globe, Skull, FolderKanban, X, Settings2, Puzzle, Zap } from "lucide-svelte";
  import { activeProjects, activateFocus, deactivateFocus, isFocused, focusState, loadProjects } from "$lib/stores/projects.js";
  import {
    settingsResults, searchSettings, clearSettingsResults,
    reloadSettingsIndex, openSettingsDeepLink,
  } from "$lib/stores/settingsSearch.js";
  import WaypointerSettingInline from "./WaypointerSettingInline.svelte";
  import WaypointerResult from "./waypointer/WaypointerResult.svelte";
  import WaypointerAskPane from "./waypointer/WaypointerAskPane.svelte";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import {
    askMode,
    ask as askAgent,
    escalate,
    resetAsk,
    loadAskCapability,
  } from "$lib/stores/waypointerAsk";
  import WaypointerInlinePreview, {
    type SpecialMode,
  } from "./waypointer/WaypointerInlinePreview.svelte";
  import {
    recentAppsStore, recentFilesStore,
    loadRecents, recordAppLaunch, openRecentFile, clearRecents,
    type RecentFile,
  } from "$lib/stores/waypointerRecents.js";
  import {
    File as FileIcon, History, Power,
    Moon, Lock, RotateCw, LogOut,
  } from "lucide-svelte";
  import {
    powerResults, updatePowerResults, clearPowerResults, invokePowerAction,
    type PowerActionResult,
  } from "$lib/stores/waypointerPower.js";
  import {
    quickActionResults,
    updateQuickActionResults,
    clearQuickActionResults,
    invokeQuickAction,
    type QuickActionResult,
  } from "$lib/stores/waypointerQuickActions.js";
  import { searchRefusals, beginSearch, noteRefusal } from "$lib/stores/searchRefusal";
  import {
    fileResults, updateFileResults, clearFileResults, openFileResult,
    type FileResult,
  } from "$lib/stores/waypointerFiles.js";
  import {
    clipboardResults, clipboardEnabled, refreshClipboardEnabled,
    updateClipboardResults, clearClipboardResults,
    copyClipboardEntry, deleteClipboardEntry, clearAllClipboard,
    type ClipboardResult,
  } from "$lib/stores/waypointerClipboard.js";
  import {
    dictResults, updateDictResults, clearDictResults,
    type DictResult,
  } from "$lib/stores/waypointerDict.js";
  import {
    extensionResults, updateExtensionResults, clearExtensionResults,
    runExtensionResult, type ExtensionResult,
  } from "$lib/stores/waypointerExtensions.js";
  import {
    shortcutResults, updateShortcutResults, clearShortcutResults,
    runShortcutResult, type ShortcutResult,
  } from "$lib/stores/waypointerShortcuts";
  import {
    refreshFromDaemon as refreshModuleWorkers,
    installListener as installModuleListener,
    searchModules,
  } from "$lib/modules/moduleSearchStore.js";
  import type { SearchResult as ModuleSearchResult } from "$lib/modules/postmsg.js";
  import {
    FileText, FileCode, FileCog, FileImage, FileArchive, FileAudio, FileVideo,
    Clipboard, Trash2,
  } from "lucide-svelte";

  let query = $state("");

  /// The element the shrinking card vacates into, kept so it can be forced to
  /// repaint.
  let backdropEl: HTMLDivElement | undefined = $state();

  /// The card itself, which is the element that shrinks - and, measurably, the
  /// one holding most of the stale paint.
  let cardEl: HTMLElement | undefined = $state();

  // Repaint the backdrop whenever the query changes, because the webview will not.
  //
  // PR-20, measured rather than argued: filter the list and the strip the card
  // gives up keeps the pixels last drawn there. The compositor is not at fault -
  // a build that composited every frame whole showed the same strips - and they
  // are not in the document either, since the keyboard walks straight past them.
  // What is left is WebKit's own painting: it repaints what it thinks is dirty,
  // the vacated region is not in that set, and the page behind it is transparent
  // so nothing else covers it.
  //
  // The nudge is on the background, NOT on opacity, and that distinction is the
  // whole of a wasted image build. `.wp-backdrop` carries
  // `animation: wp-backdrop-fade ... both`, so its `to { opacity: 1 }` stays
  // applied forever - and a CSS animation outranks an inline style, so writing
  // `el.style.opacity` did nothing at all. Same `fill-mode: both` that pinned the
  // card's transform, twice in one file.
  //
  // The animation touches only opacity, so the background is free. Swapping in a
  // fully transparent gradient and back invalidates the backdrop's background
  // paint across its whole area - which is the area the card vacates - without
  // changing a pixel of what it looks like or promoting a layer.
  //
  // Both the backdrop AND the card, because nudging the backdrop alone moved the
  // symptom only partly: the band OUTSIDE the old card box faded almost away,
  // while the rows inside it stood unchanged. An element repaints what it owns,
  // and the region inside the old box belongs to the card - which is also the
  // element that shrank, so its own paint is the stale part. The box still
  // visible at the old size IS the previous card.
  //
  // A workaround, and labelled as one: the right fix lives in the engine, and
  // this makes the shipped surface correct until that arrives.
  // The backdrop spans the whole surface, so it is the element that covers every
  // region the card can ever have occupied. `clearSurface` holds the measured
  // steps and the reasoning; it lives in `$lib/surface-clear` because this is a
  // property of every transparent shell surface and not of the waypointer, and
  // the consent surface showed what happens to the one that does not do it.
  $effect(() => {
    query;
    clearSurface(backdropEl);
  });
  let inputRef = $state<HTMLInputElement | null>(null);
  let listRef = $state<HTMLElement | null>(null);
  let commandValue = $state("");

  // Projects sorted by recent access, limited to 3 without query.
  // In "p:" prefix mode, show all matching with no limit.
  const filteredProjects = $derived((() => {
    const sorted = [...$activeProjects].sort(
      (a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0)
    );
    const trimmed = query.trim().toLowerCase();
    if (trimmed.startsWith("p:")) {
      const filter = trimmed.slice(2).trim();
      if (!filter) return sorted;
      return sorted.filter(
        (p) => p.name.toLowerCase().includes(filter) || p.rootPath.toLowerCase().includes(filter)
      );
    }
    if (!query) return sorted.slice(0, 3);
    const q = trimmed;
    return sorted.filter(
      (p) => p.name.toLowerCase().includes(q) || p.rootPath.toLowerCase().includes(q)
    );
  })());

  // App search results from Rust (max 20, pre-filtered, icons included).
  const searchResults = writable<AppEntry[]>([]);
  // Tier 2 sandboxed module search results, aggregated across all
  // workers in `moduleSearchStore`. The worker iframes themselves
  // live under a body-level hidden host owned by the store, not
  // under this component's DOM.
  const moduleResults = writable<ModuleSearchResult[]>([]);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  // Full app list cached for the calculator fallback.
  let allApps: AppEntry[] = [];

  // Init: runs once when the component mounts.
  let _initialized = false;
  $effect(() => {
    if (_initialized) return;
    _initialized = true;
    console.time("wp-init");
    initWaypointerListeners();
    console.timeLog("wp-init", "listeners");
    // Pre-load data that doesn't change during the shell session.
    fetchAllApps()
      .then((apps) => {
        console.timeLog("wp-init", `apps loaded (${apps.length})`);
        allApps = apps;
        searchResults.set(apps);
        console.timeEnd("wp-init");
      })
      .catch(() => { console.timeEnd("wp-init"); });
    reloadSettingsIndex();
    // Prime the clipboard opt-in flag so the Waypointer knows
    // whether to render the Clear-All affordance below.
    refreshClipboardEnabled();
  });

  function doSearch(q: string) {
    if (!q.trim()) {
      searchResults.set(allApps.slice(0, 8));
      return;
    }
    const t0 = performance.now();
    searchApps(q)
      .then((r) => {
        console.log(`[wp-search] apps: ${(performance.now() - t0).toFixed(1)}ms (${r.length} results)`);
        searchResults.set(r);
      })
      .catch(() => {
        searchResults.set([]);
        noteRefusal();
      });
  }

  /// Debounce delay for search fan-out. 120ms matches the input poll
  /// tick (150ms, see `$effect` further down) so typing a burst doesn't
  /// fire three invokes per keystroke. Previously doSearch ran
  /// synchronously here but updateWindowResults + searchSettings fired
  /// unconditionally on every call, causing backend pile-up.
  const SEARCH_DEBOUNCE_MS = 120;

  function debouncedSearch(q: string) {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      console.time("wp-search-total");
      // One question, one set of answers about it.
      beginSearch();
      doSearch(q);
      const t0 = performance.now();
      updateWindowResults(q);
      console.log(`[wp-search] windows: ${(performance.now() - t0).toFixed(1)}ms`);
      const t1 = performance.now();
      searchSettings(q);
      console.log(`[wp-search] settings: ${(performance.now() - t1).toFixed(1)}ms`);
      // Power-plugin via the generic manager bridge. Previously
      // missing here — Power plugin was registered but never queried,
      // so typing "shutdown" returned nothing. Fires the same
      // debounced cycle as apps / windows / settings.
      const t2 = performance.now();
      updatePowerResults(q)
        .then(() => {
          console.log(
            `[wp-search] power: ${(performance.now() - t2).toFixed(1)}ms`,
          );
        })
        .catch(() => noteRefusal());
      // Quick-Actions plugin: same generic-bridge pattern. Catalog
      // covers DND, network/BT toggles, theme switches, Settings
      // launchers (Sprint D).
      const t2a = performance.now();
      updateQuickActionResults(q)
        .then(() => {
          console.log(
            `[wp-search] quick-actions: ${(performance.now() - t2a).toFixed(1)}ms`,
          );
        })
        .catch(() => noteRefusal());
      // File-search plugin: same bridge, separate section.
      const t3 = performance.now();
      updateFileResults(q)
        .then(() => {
          console.log(
            `[wp-search] files: ${(performance.now() - t3).toFixed(1)}ms`,
          );
        })
        .catch(() => noteRefusal());
      // Clipboard-history plugin: only fires when opt-in is on.
      // Backend short-circuits to empty when disabled; we skip the
      // invoke entirely in that case to save the IPC hop.
      const t4 = performance.now();
      updateClipboardResults(q)
        .then(() => {
          console.log(
            `[wp-search] clipboard: ${(performance.now() - t4).toFixed(1)}ms`,
          );
        })
        .catch(() => noteRefusal());
      // Dictionary plugin: also via the generic bridge. Returns empty
      // until the WordNet data is loaded (first query kicks off the
      // background load, usually ready by the second keystroke).
      const t5 = performance.now();
      updateDictResults(q)
        .then(() => {
          console.log(
            `[wp-search] dict: ${(performance.now() - t5).toFixed(1)}ms`,
          );
        })
        .catch(() => noteRefusal());
      // The installed Tier 1 extensions, which nothing asked until now: the
      // aggregate command that reaches the module runtime has no caller, so
      // every module in the tree was unreachable from here.
      const t6 = performance.now();
      updateExtensionResults(q)
        .then(() => {
          console.log(
            `[wp-search] extensions: ${(performance.now() - t6).toFixed(1)}ms`,
          );
        })
        .catch(() => noteRefusal());
      // The focused app's own quick actions. Every layer under this was built
      // and nothing rendered it, so an app could publish "Run tests" and no
      // surface would list it.
      updateShortcutResults(q).catch(() => noteRefusal());
      // The Tier 2 workers. This call was cut during the same bisection as
      // the worker pool, and the pool came back on 8 September once the
      // stretch it was blamed for failed to reproduce - so the cut is over
      // and this fans the query out to every mounted worker. A pool with no
      // workers resolves empty, which is the state on any machine today:
      // nothing mounts a worker unless a Tier 2 module is installed.
      searchModules(q)
        .then((results) => {
          moduleResults.set(results);
        })
        .catch(() => {
          moduleResults.set([]);
        });
      requestAnimationFrame(() => {
        console.timeEnd("wp-search-total");
      });
    }, SEARCH_DEBOUNCE_MS);
  }

  function open() {
    console.time("wp-open");
    // Re-load projects on every Waypointer open. If the Knowledge
    // daemon wasn't running at shell startup, this is the retry that
    // picks up newly-available data without a shell restart.
    loadProjects();
    query = "";
    commandValue = "";
    inlineResult.set(null);
    specialMode.set(null);
    specialArg.set("");
    clearWindowResults();
    clearProcessResults();
    clearSettingsResults();
    clearPowerResults();
    clearQuickActionResults();
    clearFileResults();
    clearClipboardResults();
    clearDictResults();
    clearExtensionResults();
    clearShortcutResults();
    clearRecents();
    console.timeLog("wp-open", "stores cleared");
    // Load MRU apps + graph-recent files in parallel. Both are cached
    // behind short TTLs on the Rust side so repeated opens are cheap.
    loadRecents(allApps).then(() => {
      console.timeLog("wp-open", "recents loaded");
    }).catch(() => {});
    // Keep the empty-query grid EMPTY — recents now fill that role.
    // Previously we showed `allApps.slice(0, 8)` as generic suggestions;
    // with recents populated, alphabetical-first-8 is worse than MRU.
    searchResults.set([]);
    console.timeLog("wp-open", `set ${Math.min(8, allApps.length)}/${allApps.length} apps`);
    if (listRef) listRef.scrollTop = 0;
    // Measure when the browser actually paints.
    requestAnimationFrame(() => {
      console.timeEnd("wp-open");
    });
  }

  // Watch visibility and call open() when shown.
  let _visUnsub: (() => void) | null = null;
  $effect(() => {
    if (_visUnsub) return;
    _visUnsub = waypointerVisible.subscribe((visible) => {
      if (visible) open();
    });
    return () => { _visUnsub?.(); _visUnsub = null; };
  });

  function close() {
    resetAsk();
    closeWaypointer();
  }

  let kbActive = $state(false);
  let lastMouse = { x: 0, y: 0 };

  function handleKeydown(e: KeyboardEvent) {
    // Ask mode (waypointer-ai-prompt.md): Tab flips the input into a prompt, the
    // answer streams inline. First Esc drops back to search (the session
    // persists server-side); the render + fixture are here, the real agent call
    // is the coder's `waypointer_ask` seam.
    let asking = false;
    askMode.subscribe((v) => { asking = v; })();
    if (e.key === "Tab" && !asking) {
      e.preventDefault();
      askMode.set(true);
      void loadAskCapability();
      return;
    }
    if (asking) {
      if (e.key === "Escape") {
        e.preventDefault();
        resetAsk();
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        void askAgent(query);
        query = "";
        return;
      }
      if (e.key.toLowerCase() === "j" && e.ctrlKey) {
        e.preventDefault();
        void escalate();
        close();
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        return;
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === "Enter") {
      // Check special modes first.
      let mode: SpecialMode = null;
      let arg = "";
      specialMode.subscribe((v) => { mode = v; })();
      specialArg.subscribe((v) => { arg = v; })();

      if (mode === "shell" && arg) {
        e.preventDefault();
        e.stopPropagation();
        runShellCommand(arg, e.shiftKey);
        return;
      }
      if (mode === "man" && arg) {
        e.preventDefault();
        openManPage(arg);
        return;
      }
      if (mode === "url" && arg) {
        e.preventDefault();
        openUrlAction(arg);
        return;
      }
      if (mode === "search" && arg) {
        e.preventDefault();
        webSearchAction(arg);
        return;
      }
      if (mode === "kill" && e.shiftKey) {
        // Shift+Enter in kill mode: SIGKILL the selected process.
        e.preventDefault();
        e.stopPropagation();
        let procs: ProcessInfo[] = [];
        processResults.subscribe((v) => { procs = v; })();
        // The selected process is whichever has data-selected in the DOM.
        const selected = document.querySelector("[data-slot='command-item'][data-selected]");
        const selectedValue = selected?.getAttribute("data-value") ?? "";
        if (selectedValue.startsWith("proc-")) {
          const pid = parseInt(selectedValue.slice(5), 10);
          const proc = procs.find((p) => p.pid === pid);
          if (proc) { killProcessAction(proc, true); return; }
        }
        // Fallback: kill first match.
        if (procs.length > 0) { killProcessAction(procs[0], true); return; }
      }

      // Check inline math/unit result.
      let r: InlineEvalResult | null = null;
      inlineResult.subscribe((v) => { r = v; })();
      if (r) {
        e.preventDefault();
        handleInlineAction(r);
        return;
      }
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      kbActive = true;
    }
  }

  function handleGlobalMouseMove(e: MouseEvent) {
    if (!kbActive) return;
    const dx = e.clientX - lastMouse.x;
    const dy = e.clientY - lastMouse.y;
    lastMouse.x = e.clientX;
    lastMouse.y = e.clientY;
    // Only exit keyboard mode if mouse actually moved significantly.
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) {
      kbActive = false;
    }
  }

  // Types live with WaypointerInlinePreview, which renders the card
  // these stores drive.
  const inlineResult = writable<InlineEvalResult | null>(null);

  /// The message id for an action from this surface that would not run, or null.
  ///
  /// Held here rather than raised as a toast, and the reason is not the one this
  /// comment used to give. It said a toast "renders in THIS window and this
  /// window is the one that closes" - but `raiseRefusal` emits `arlen://toast`,
  /// which the top bar's bridge renders, and the quick actions on this very
  /// surface have used it that way since it existed. A toast would be seen.
  ///
  /// The real reason to keep it here is better than the wrong one: the launcher
  /// stays OPEN on a failure, so the sentence sits beside the thing that refused
  /// and the person can try again without re-opening anything. A toast puts the
  /// news on the desktop and takes the surface away.
  ///
  /// Named for module actions until 6 September, when three more of this
  /// surface's actions turned out to need it - two clipboard copies and a
  /// process kill, each of which did its thing, ignored the answer and closed.
  const actionError = writable<string | null>(null);

  const specialMode = writable<SpecialMode>(null);
  const specialArg = writable<string>("");

  /// The four special-mode actions, and every one of them CLOSES ONLY IF IT WORKED.
  ///
  /// They used to fire the call and close on the same tick, so a command that
  /// could not start, a man page with no `man`, a URL that would not open or a
  /// search that never left said nothing at all - the launcher was already gone,
  /// and a toast renders in this window, which is the one being hidden. The
  /// module-result branch below has awaited its own three since it was written
  /// and says exactly that in its comment; these four had not been brought over.
  async function actOrSay(run: () => Promise<unknown>, message: string) {
    try {
      await run();
    } catch {
      actionError.set(message);
      return;
    }
    actionError.set(null);
    close();
  }

  function runShellCommand(cmd: string, inTerminal: boolean) {
    void actOrSay(() => executeShellCommand(cmd, inTerminal), "sh.wp.errRun");
  }

  function openManPage(topic: string) {
    void actOrSay(() => executeShellCommand(`man ${topic}`, true), "sh.wp.errRun");
  }

  function openUrlAction(url: string) {
    void actOrSay(() => openUrl(url), "sh.wp.errOpenUrl");
  }

  function webSearchAction(query: string) {
    // Its own sentence, not the link one. A web search does open a URL, so
    // sharing `errOpenUrl` was defensible and still wrong to read: the person
    // typed a question, not a link, and being told "that link could not be
    // opened" describes something they did not do.
    void actOrSay(() => webSearch(query), "sh.wp.errSearch");
  }

  /// Map `PowerActionResult.id` to its lucide icon. The backend sets
  /// a freedesktop icon name on `SearchResult.icon` (`system-suspend`,
  /// `system-reboot`, …) but those aren't guaranteed to be present in
  /// the user's icon theme. Rest of the shell uses lucide consistently
  /// for chrome-icons, so we keep that convention here.
  /// eslint-disable-next-line @typescript-eslint/no-explicit-any
  function iconForPowerAction(id: string): any {
    switch (id) {
      case "power.sleep":    return Moon;
      case "power.lock":     return Lock;
      case "power.restart":  return RotateCw;
      case "power.shutdown": return Power;
      case "power.logout":   return LogOut;
      default:               return Power;
    }
  }

  /// Map the lucide-icon-name string returned by the `core.files`
  /// plugin (`file-code`, `file-text`, …) to the actual lucide-svelte
  /// component. Backend picks the name from file extension; frontend
  /// renders the corresponding lucide icon so we keep the chrome
  /// palette uniform without shipping extension->icon maps in both
  /// languages.
  /// eslint-disable-next-line @typescript-eslint/no-explicit-any
  function iconForFileName(icon: string | null): any {
    switch (icon) {
      case "file-code":    return FileCode;
      case "file-text":    return FileText;
      case "file-cog":     return FileCog;
      case "file-image":   return FileImage;
      case "file-archive": return FileArchive;
      case "file-audio":   return FileAudio;
      case "file-video":   return FileVideo;
      default:             return FileIcon;
    }
  }

  function killProcessAction(proc: ProcessInfo, force: boolean) {
    // The same shape, and the one where silence costs most: a kill that was
    // refused - no permission, or the process already gone - closed the launcher
    // as though it had worked, and the only way to find out was to come back and
    // look for the process again.
    void killProcess(proc.pid, force)
      .then(() => {
        actionError.set(null);
        close();
      })
      .catch(() => actionError.set("sh.wp.errKill"));
  }

  /// Checks if a string looks like a URL.
  function looksLikeUrl(s: string): boolean {
    if (/^https?:\/\//i.test(s)) return true;
    // domain.tld pattern (at least one dot, TLD 2-10 chars, no spaces)
    if (/^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z]{2,10})+([\/\?#].*)?$/i.test(s)) return true;
    return false;
  }

  // The Tier 2 worker pool. This was off behind a `void` and a comment saying
  // "PERMANENTLY OFF until the worker pool can be initialised without taking
  // over the Waypointer's flex layout" - the card was said to stretch to fill
  // the window once the listener installed.
  //
  // MEASURED on 8 September rather than inherited. With a Tier 2 module present
  // (`?searchmock=tier2`) the host element and its iframe mount and the card is
  // 600x74, the same to the pixel as with the pool off. The stretch does not
  // reproduce, so the caller is on and the sentence is gone; what remains of
  // that finding is the geometry assertion in `drive-launcher.sh`, which fails
  // if the card ever grows with a worker mounted.
  //
  // The bound on that measurement, because it is not nothing: it is a headless
  // WebKit render of the page, not the layer-shell overlay anchored to all four
  // edges that the original sentence named. If the stretch is specific to the
  // anchored surface it would take a VM boot to see. It is safe to be wrong
  // here today - nothing mounts a worker unless a Tier 2 module is installed,
  // and no module ships on the image at all yet.
  $effect(() => {
    installModuleListener();
    refreshModuleWorkers();
    const handle = setInterval(() => {
      refreshModuleWorkers();
    }, 30_000);
    return () => clearInterval(handle);
  });

  // Poll for query changes and trigger search + evaluation.
  // The interval lives inside `$effect` so each mount gets its own
  // handle and the effect's cleanup reliably tears it down on
  // unmount/HMR. The previous module-scoped guard could leak the
  // interval when the effect ran twice before cleanup fired.
  $effect(() => {
    let prev = "";
    const pollInterval = setInterval(() => {
        const q = inputRef?.value ?? query;
        if (q === prev) return;
        prev = q;
        // A refusal is about the thing that just failed, and typing is moving on.
        // It used to survive a new query because the four actions that could
        // refuse all CLOSED the launcher, so nobody ever saw one over a fresh
        // search; keeping the window open on a failure - which is the point of
        // saying anything at all - made a stale sentence reachable, so it goes
        // when the question changes.
        actionError.set(null);
        let trimmed = q.trim();
        let searchQuery = q;

        // The `unicode ` keyword is a DOOR onto the module's `u:` search, not a
        // second implementation of it. It was its own command, its own store and
        // its own result section, so the same question answered differently
        // depending on which way it was asked - and neither answer was wrong
        // enough to look wrong. The keyword stays because it is more
        // discoverable than a prefix and nobody should have to relearn what they
        // already knew; what it does now is rewrite the query and fall through.
        if (/^unicode\b/i.test(trimmed)) {
          trimmed = `u:${trimmed.slice(7).trim()}`;
          searchQuery = trimmed;
        }

        // Detect special prefixes.
        if (trimmed.startsWith(">")) {
          const cmd = trimmed.slice(1).trim();
          specialMode.set("shell");
          specialArg.set(cmd);
          searchResults.set([]);
          inlineResult.set(null);
          // DOM: show shell result.
          const wrap = document.getElementById("wp-inline-wrap");
          const el = document.getElementById("wp-inline-result");
          const hint = document.getElementById("wp-inline-hint");
          if (wrap) { wrap.style.display = cmd ? "" : "none"; wrap.style.paddingBottom = "8px"; }
          if (el) el.textContent = cmd || $t("sh.wp.typeCommand");
          if (hint) hint.textContent = $t("sh.wp.runHint");
          // Hide the empty list.
          const list = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (list) list.style.display = "none";
          return;
        }
        if (trimmed.startsWith("#")) {
          const topic = trimmed.slice(1).trim();
          specialMode.set("man");
          specialArg.set(topic);
          searchResults.set([]);
          inlineResult.set(null);
          const wrap = document.getElementById("wp-inline-wrap");
          const el = document.getElementById("wp-inline-result");
          const hint = document.getElementById("wp-inline-hint");
          if (wrap) { wrap.style.display = topic ? "" : "none"; wrap.style.paddingBottom = "8px"; }
          if (el) el.textContent = topic ? `man ${topic}` : $t("sh.wp.typeTopic");
          if (hint) hint.textContent = $t("sh.wp.manualHint");
          const list2 = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (list2) list2.style.display = "none";
          return;
        }

        // "?" prefix: web search.
        if (trimmed.startsWith("?")) {
          const searchQuery = trimmed.slice(1).trim();
          specialMode.set("search");
          specialArg.set(searchQuery);
          searchResults.set([]);
          inlineResult.set(null);
          const wrap = document.getElementById("wp-inline-wrap");
          const el = document.getElementById("wp-inline-result");
          const hint = document.getElementById("wp-inline-hint");
          if (wrap) { wrap.style.display = searchQuery ? "" : "none"; wrap.style.paddingBottom = "8px"; }
          if (el) el.textContent = searchQuery ? `Search: ${searchQuery}` : "Type to search the web...";
          if (hint) hint.textContent = "Search DuckDuckGo";
          const listS = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (listS) listS.style.display = "none";
          return;
        }

        // "kill" keyword: process list.
        if (trimmed.toLowerCase().startsWith("kill")) {
          const filter = trimmed.slice(4).trim();
          specialMode.set("kill");
          specialArg.set(filter);
          searchResults.set([]);
          inlineResult.set(null);
          updateProcessResults(filter);
          // Hide inline wrap, show list.
          const wrap = document.getElementById("wp-inline-wrap");
          if (wrap) wrap.style.display = "none";
          const listK = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (listK) listK.style.display = "";
          return;
        }

        // URL detection: if it looks like a URL, show "Open URL".
        if (looksLikeUrl(trimmed)) {
          specialMode.set("url");
          specialArg.set(trimmed);
          searchResults.set([]);
          inlineResult.set(null);
          const wrap = document.getElementById("wp-inline-wrap");
          const el = document.getElementById("wp-inline-result");
          const hint = document.getElementById("wp-inline-hint");
          if (wrap) { wrap.style.display = ""; wrap.style.paddingBottom = "8px"; }
          if (el) el.textContent = trimmed;
          if (hint) hint.textContent = "Open link";
          const listU = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (listU) listU.style.display = "none";
          return;
        }

        // "p:" prefix: project search.
        if (trimmed.toLowerCase().startsWith("p:")) {
          const filter = trimmed.slice(2).trim();
          specialMode.set("projects");
          specialArg.set(filter);
          searchResults.set([]);
          inlineResult.set(null);
          clearProcessResults();
          const wrap = document.getElementById("wp-inline-wrap");
          if (wrap) wrap.style.display = "none";
          const listP = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (listP) listP.style.display = "";
          return;
        }

        // Normal mode: clear special state.
        specialMode.set(null);
        specialArg.set("");
        clearProcessResults();
        // Restore list visibility.
        const listEl = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
        if (listEl) listEl.style.display = "";

        // Search apps in Rust.
        debouncedSearch(searchQuery);
        // Evaluate math/units.
        if (trimmed.length < 2) {
          inlineResult.set(null);
          return;
        }
        const evalT0 = performance.now();
        evaluateInput(q)
          .then((r) => {
            console.log(`[wp-search] evaluate: ${(performance.now() - evalT0).toFixed(1)}ms`);

            inlineResult.set(r);
            // DOM fallback: bypass Svelte reactivity.
            const el = document.getElementById("wp-inline-result");
            const wrap = document.getElementById("wp-inline-wrap");
            const list = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
            if (r) {
              if (el) el.textContent = r.display;
              const hint = document.getElementById("wp-inline-hint");
              if (hint) {
                // Only promise a calculator when one actually
                // resolves — Enter on the error row launches the
                // first app matching these names and silently does
                // nothing otherwise.
                const calcAvailable = allApps.some((a) =>
                  a.name.toLowerCase().includes("calculator") ||
                  a.name.toLowerCase().includes("rechner"));
                hint.textContent = r.result_type === "error"
                  ? (calcAvailable ? "Open Calculator" : "")
                  : "Copy";
              }
              if (wrap) wrap.style.display = "";
              // Hide list and add padding when no app results visible.
              const hasItems = list?.querySelector("[data-slot='command-item']");
              if (wrap) wrap.style.paddingBottom = hasItems ? "2px" : "8px";
              if (list) list.style.display = hasItems ? "" : "none";
            } else {
              if (wrap) { wrap.style.display = "none"; wrap.style.paddingBottom = ""; }
              if (list) list.style.display = "";
            }
          })
          .catch(() => {
            inlineResult.set(null);
            const wrap = document.getElementById("wp-inline-wrap");
            if (wrap) wrap.style.display = "none";
          });
      }, 150);
    return () => clearInterval(pollInterval);
  });

  /// Click dispatch for the inline preview card — the per-mode
  /// actions stay here with the rest of the domain wiring.
  function activateInlinePreview() {
    let mode: SpecialMode = null;
    let arg = "";
    specialMode.subscribe((v) => { mode = v; })();
    specialArg.subscribe((v) => { arg = v; })();
    if (mode === "shell" && arg) { runShellCommand(arg, false); return; }
    if (mode === "man" && arg) { openManPage(arg); return; }
    if (mode === "url" && arg) { openUrlAction(arg); return; }
    if (mode === "search" && arg) { webSearchAction(arg); return; }
    const r = $inlineResult;
    if (r) handleInlineAction(r);
  }

  function handleInlineAction(result: InlineEvalResult) {
    if (result.result_type === "error") {
      // Launch a calculator app from the index.
      const calc = allApps.find((a) =>
        a.name.toLowerCase().includes("calculator") ||
        a.name.toLowerCase().includes("rechner")
      );
      if (calc) {
        // Also with its id: this is an entry from the index like any other, and
        // a launch without the id can never be raised instead.
        launchAppCmd(calc.exec, calc.app_id, calc.name);
      }
      close();
    } else {
      void navigator.clipboard
        .writeText(result.copy_value)
        .then(() => {
          actionError.set(null);
          close();
        })
        .catch(() => actionError.set("sh.wp.errCopy"));
    }
  }

  function launchAppAndClose(app: AppEntry) {
    // Record the launch BEFORE closing: the close path may tear down
    // event listeners, and the record call is fire-and-forget so it
    // doesn't block the actual launch below.
    recordAppLaunch(app.exec);
    // With the id, not just the exec: it is what tells the shell this is an app
    // it already has a window for, and a launch without it always starts a
    // second process (`app-instance-model.md`).
    launchAppCmd(app.exec, app.app_id, app.name);
    close();
  }

  function openRecentFileAndClose(file: RecentFile) {
    openRecentFile(file.path);
    close();
  }

  /// Shortened display name for a recent-file path. Shows the final
  /// path component + parent directory so two files with the same
  /// name in different dirs stay distinguishable.
  function shortPath(p: string): string {
    const parts = p.split("/").filter((x) => x.length > 0);
    if (parts.length === 0) return p;
    if (parts.length === 1) return parts[0];
    return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
  }

  function switchToWindow(win: WindowInfo) {
    activateWindow(win.id);
    close();
  }

  /// Looks up the app icon (base64 data URL) by app_id or exec name.
  function appIconFor(name: string): string | null {
    const lower = name.toLowerCase();
    const app = allApps.find((a) =>
      a.icon_name.toLowerCase() === lower ||
      a.exec.toLowerCase().split(/\s/)[0].endsWith(lower)
    );
    return app?.icon_data ?? null;
  }
</script>

<svelte:window onkeydown={handleKeydown} onmousemove={handleGlobalMouseMove} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="wp-backdrop" bind:this={backdropEl} onclick={close}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- A `main`, not a div: this card is the whole content of the launcher window,
       and until now nothing in that window was inside a landmark at all. The
       heading names the window for a screen reader landing in it - hidden,
       because the search field it opens on already says what this is. -->
  <main class="wp-card shell-surface" bind:this={cardEl} onclick={(e) => e.stopPropagation()}>
    <h1 class="sr-only">{$t("sh.app.title.waypointer")}</h1>
    <Command class="wp-root" shouldFilter={false} bind:value={commandValue}>
      <!-- The Ask-mode marker: a quiet chip riding the input line, exactly where
           the user types (stronger than a frame, which reads as mere focus). -->
      <div class="wp-input-wrap">
        <CommandInput
          placeholder={$askMode ? $t("sh.wp.askPlaceholder") : $t("sh.wp.searchPlaceholder")}
          bind:value={query}
          bind:ref={inputRef}
          autofocus
        />
        {#if $askMode}
          <span class="wp-ask-chip"><Badge variant="outline">{$t("sh.wp.agent")}</Badge></span>
        {/if}
      </div>

      {#if $askMode}
        <WaypointerAskPane />
      {/if}

      <!-- The search machinery stays mounted while asking (the 150ms poll writes
           into it by id and must not lose its targets); it is hidden with a
           hard class instead. -->
      <div class:wp-hidden={$askMode}>
        <WaypointerInlinePreview
          {specialMode}
          {inlineResult}
          onActivate={activateInlinePreview}
        />
      </div>

      <!-- OUTSIDE THE LIST, and that is the whole point of moving it here. It
           used to render inside `CommandList`, and an inline result HIDES that
           list - `list.style.display = "none"` in the evaluate handler, when
           there are no app rows behind the card. So a copy that failed set this
           store, rendered this sentence, and put it in a container nobody could
           see: the launcher stayed open showing the result with no word about
           the refusal, which is the same silence the store was added to end.
           Measured on 10 September through `waypointer-refuses-copy`: the text
           was in the DOM (`Das ließ sich nicht kopieren.`) with zero client
           rects. -->
      {#if $actionError}
        <div class="wp-empty wp-action-error">{$t($actionError)}</div>
      {/if}

      <CommandList
        class="wp-list {kbActive ? 'wp-kb-active' : ''} {$askMode ? 'wp-hidden' : ''}"
        bind:ref={listRef}
      >
        <!-- CommandEmpty is unusable with shouldFilter={false} because
             cmdk always reports 0 internal matches. Use our own check
             across all provider stores instead. -->

        {#if !$inlineResult && $searchResults.length === 0 && $windowResults.length === 0 && $settingsResults.length === 0 && $powerResults.length === 0 && $quickActionResults.length === 0 && $fileResults.length === 0 && $clipboardResults.length === 0 && $dictResults.length === 0 && $extensionResults.length === 0 && filteredProjects.length === 0 && $recentAppsStore.length === 0 && $recentFilesStore.length === 0 && query.trim().length > 0}
          <!-- Two different sentences, because they are two different facts.
               Every provider's failure leaves its store empty, which is also
               what finding nothing looks like, so without the count this said
               "No results found." to a person whose backend was down. -->
          <div class="wp-empty">
            {$searchRefusals > 0 ? $t("sh.wp.searchRefused") : $t("sh.wp.noResults")}
          </div>
        {/if}

        <!-- Power actions from the `core.power` plugin. Placed above
             the app-search group so an exact keyword like "shutdown"
             (relevance 1.0) surfaces at the top, beating any partial
             app-name match. -->
        {#if $powerResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.system")}>
            {#each $powerResults as action (action.id)}
              {@const ActionIcon = iconForPowerAction(action.id)}
              <CommandItem
                value={`power-${action.id}`}
                onSelect={() => {
                  invokePowerAction(action);
                  close();
                }}
              >
                <WaypointerResult
                  icon={ActionIcon}
                  title={action.title_key ? $t(action.title_key) : action.title}
                  description={action.description_key
                    ? $t(action.description_key)
                    : action.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        <!-- Quick Actions (DND, theme, settings shortcuts, …) from
             the `core.quick_actions` plugin. Priority 50 keeps them
             below Apps for app-name queries, but for Arlen-specific
             keywords (DND / brightness / focus / …) Apps has nothing
             to offer so Quick-Actions surface naturally. Toast-after-
             execute confirmation arrives via the `arlen://toast`
             event bridge in `+layout.svelte`. -->
        {#if $quickActionResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.quickActions")}>
            {#each $quickActionResults as qa (qa.id)}
              <CommandItem
                value={`qa-${qa.id}`}
                onSelect={() => {
                  // The same words the row shows, so a refusal names what was
                  // picked rather than `qa.toggle_wifi`.
                  invokeQuickAction(qa.id, qa.title_key ? $t(qa.title_key) : qa.title);
                  close();
                }}
              >
                <WaypointerResult
                  icon={Settings2}
                  title={qa.title_key ? $t(qa.title_key) : qa.title}
                  description={qa.description_key
                    ? $t(qa.description_key)
                    : qa.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        <!-- Empty-query landing page: MRU apps + graph-recent files.
             Hidden as soon as the user types anything so the search
             result sections can take over. -->
        {#if query.trim().length === 0 && $recentAppsStore.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.recentApps")}>
            {#each $recentAppsStore as app, i (app.name + "_rec_" + i)}
              <CommandItem
                value={`recent-app-${app.exec}`}
                onSelect={() => launchAppAndClose(app)}
              >
                <WaypointerResult
                  iconUrl={app.icon_data}
                  fallbackIcon={AppWindow}
                  title={app.name}
                  description={app.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          {#if $recentFilesStore.length > 0}
            <CommandSeparator />
          {/if}
        {/if}

        {#if query.trim().length === 0 && $recentFilesStore.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.recentFiles")}>
            {#each $recentFilesStore as file (file.path)}
              <CommandItem
                value={`recent-file-${file.path}`}
                onSelect={() => openRecentFileAndClose(file)}
              >
                <WaypointerResult
                  icon={FileIcon}
                  emphasis={60}
                  title={shortPath(file.path)}
                  description={file.path}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}

        {#if $windowResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.windows")}>
            {#each $windowResults as win (win.id)}
              {@const icon = appIconFor(win.app_id)}
              <CommandItem
                value={`window-${win.id}`}
                onSelect={() => switchToWindow(win)}
              >
                <WaypointerResult
                  iconUrl={icon}
                  badge="window"
                  fallbackIcon={AppWindow}
                  title={win.title}
                  description={win.app_id}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          {#if $searchResults.length > 0}
            <CommandSeparator />
          {/if}
        {/if}

        {#if $processResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.processes")}>
            {#each $processResults as proc (proc.pid)}
              {@const procIcon = appIconFor(proc.name)}
              <CommandItem
                value={`proc-${proc.pid}`}
                onSelect={() => killProcessAction(proc, false)}
              >
                <WaypointerResult
                  iconUrl={procIcon}
                  badge="kill"
                  fallbackIcon={Skull}
                  title={proc.name}
                  description={$t("sh.wp.procDetail", { pid: proc.pid, size: formatBytes(proc.memory_bytes, $locale) })}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}

        {#if $searchResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.applications")}>
            {#each $searchResults as app, i (app.name + i)}
              <CommandItem
                value={app.name}
                onSelect={() => launchAppAndClose(app)}
              >
                <WaypointerResult
                  iconUrl={app.icon_data}
                  fallbackIcon={AppWindow}
                  title={app.name}
                  description={app.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if filteredProjects.length > 0 || $isFocused}
          <CommandGroup heading={$t("sh.wp.grp.projects")}>
            {#if $isFocused}
              <CommandItem value="focus-exit" onSelect={() => { deactivateFocus(); close(); }}>
                <WaypointerResult
                  icon={X}
                  emphasis={60}
                  title={$t("sh.wp.exitFocus", { name: $focusState.projectName })}
                />
              </CommandItem>
            {/if}
            {#each filteredProjects as project (project.id)}
              <CommandItem value={`focus-${project.id}`} onSelect={() => { activateFocus(project); close(); }}>
                <WaypointerResult
                  icon={FolderKanban}
                  emphasis={60}
                  title={project.name}
                  description={project.rootPath}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $clipboardResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.clipboard")}>
            {#each $clipboardResults as entry (entry.id)}
              <CommandItem
                value={`clip-item-${entry.id}`}
                onSelect={() => { copyClipboardEntry(entry); close(); }}
              >
                <WaypointerResult
                  icon={Clipboard}
                  emphasis={60}
                  title={entry.title}
                  description={entry.description}
                >
                  {#snippet trailing()}
                    <button
                      class="wp-inline-btn"
                      aria-label={$t("sh.wp.removeFromHistory")}
                      onclick={(e) => { e.stopPropagation(); deleteClipboardEntry(entry); }}
                    >
                      <Trash2 size={12} strokeWidth={1.5} />
                    </button>
                  {/snippet}
                </WaypointerResult>
              </CommandItem>
            {/each}
            {#if $clipboardEnabled && $clipboardResults.length >= 2}
              <CommandItem
                value="clip-clear-all"
                onSelect={() => { clearAllClipboard(); close(); }}
              >
                <WaypointerResult
                  icon={Trash2}
                  emphasis={60}
                  title={$t("sh.wp.clearClipboard")}
                  description={$t("sh.wp.clearClipboardDesc")}
                />
              </CommandItem>
            {/if}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $fileResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.files")}>
            {#each $fileResults as file (file.id)}
              {@const FileIconComponent = iconForFileName(file.icon)}
              <CommandItem
                value={`file-${file.id}`}
                onSelect={() => { openFileResult(file); close(); }}
              >
                <WaypointerResult
                  icon={FileIconComponent}
                  emphasis={60}
                  title={file.title}
                  description={file.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $dictResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.definitions")}>
            {#each $dictResults as def (def.id)}
              <CommandItem
                value={`dict-${def.id}`}
                onSelect={() => { close(); }}
              >
                <WaypointerResult
                  icon={BookOpen}
                  emphasis={60}
                  title={def.title}
                  description={def.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $extensionResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.extensions")}>
            {#each $extensionResults as ext (ext.id)}
              <CommandItem
                value={`extension-${ext.id}`}
                onSelect={() => { void runExtensionResult(ext); close(); }}
              >
                <WaypointerResult
                  icon={Puzzle}
                  emphasis={60}
                  title={ext.title}
                  description={ext.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $shortcutResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.appActions")}>
            {#each $shortcutResults as sc (sc.id)}
              <CommandItem
                value={`shortcut-${sc.id}`}
                onSelect={() => { void runShortcutResult(sc); close(); }}
              >
                <WaypointerResult
                  icon={Zap}
                  emphasis={60}
                  title={sc.title}
                  description={sc.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $settingsResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.settings")}>
            {#each $settingsResults as sr (sr.setting.id)}
              <CommandItem
                value={`setting-${sr.setting.id}`}
                onSelect={() => {
                  openSettingsDeepLink(sr.setting.panel, sr.setting.deepLink.split('#')[1]);
                  close();
                }}
              >
                <WaypointerResult
                  icon={Settings2}
                  emphasis={60}
                  title={sr.setting.title}
                  description={sr.setting.section}
                >
                  {#snippet trailing()}
                    {#if sr.setting.inlineAction}
                      <WaypointerSettingInline
                        action={sr.setting.inlineAction}
                        {query}
                        label={sr.setting.title}
                      />
                    {/if}
                  {/snippet}
                </WaypointerResult>
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $moduleResults.length > 0}
          <CommandGroup heading={$t("sh.wp.grp.modules")}>
            {#each $moduleResults as result (result.id)}
              <CommandItem
                value={`module:${result.id}`}
                onSelect={async () => {
                  // A refusal used to be swallowed AND the launcher closed on top
                  // of it, so pressing Enter on a module result could copy
                  // nothing, open nothing or run nothing with the surface that
                  // could have said so already gone. The toast channel does not
                  // help here - it renders in this window, which is the one being
                  // hidden - so a failure keeps the launcher open and says it.
                  let ok = true;
                  if (result.action.type === "copy") {
                    try {
                      await navigator.clipboard.writeText(result.action.text);
                    } catch {
                      ok = false;
                      actionError.set("sh.wp.errCopy");
                    }
                  } else if (result.action.type === "open_url") {
                    try {
                      await invoke("open_url", { url: result.action.url });
                    } catch {
                      ok = false;
                      actionError.set("sh.wp.errOpenUrl");
                    }
                  } else if (result.action.type === "execute") {
                    try {
                      await invoke("execute_shell_command", {
                        command: result.action.command,
                        inTerminal: false,
                      });
                    } catch {
                      ok = false;
                      actionError.set("sh.wp.errRun");
                    }
                  }
                  if (ok) {
                    actionError.set(null);
                    closeWaypointer();
                  }
                }}
              >
                <WaypointerResult
                  fallbackIcon={Globe}
                  title={result.title}
                  description={result.description}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}
      </CommandList>

      <!-- Subdued footer: contextual hints in plain words. Kill mode
           explains its two shortcuts (they used to live in the group
           heading as signal names); the empty landing shows the
           prefix cheatsheet so the blank state teaches instead of
           staring back. -->
      {#if $askMode}
        <!-- The ask pane carries its own footer. -->
      {:else if $specialMode === "kill"}
        <div class="wp-footer">
          <span>{$t("sh.wp.killEnter")}</span>
          <span>{$t("sh.wp.killShiftEnter")}</span>
        </div>
      {:else if query.trim().length === 0}
        <div class="wp-footer">
          <span>{$t("sh.wp.hintCommand")}</span>
          <span>{$t("sh.wp.hintManual")}</span>
          <span>{$t("sh.wp.hintWeb")}</span>
          <span>{$t("sh.wp.hintProjects")}</span>
          <span>{$t("sh.wp.hintAgent")}</span>
        </div>
      {/if}
    </Command>
  </main>
</div>

<style>
  /* The waypointer webview is a transparent layer-shell overlay —
     the page itself must never paint. The competing declarations
     (Tailwind preflight, the app.css body rule) all live in
     @layer base, and unlayered author rules like these win over any
     layered rule regardless of specificity, so no escalation is
     needed. */
  :global(html), :global(body) {
    background: transparent;
    overflow: hidden;
    height: 100%;
  }

  /* Hard-hide the search machinery while Ask mode is up: the 150ms poll writes
     inline display values by id, so only !important wins over it. */
  :global(.wp-hidden) {
    display: none !important;
  }

  .wp-input-wrap {
    position: relative;
  }
  .wp-ask-chip {
    position: absolute;
    inset-inline-end: 0.9rem;
    top: 50%;
    transform: translateY(-50%);
    pointer-events: none;
  }

  .wp-backdrop {
    position: fixed;
    inset: 0;
    z-index: 0;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 25vh;
    background: var(--color-bg-overlay);
    overflow: hidden;
    animation: wp-backdrop-fade var(--duration-fast, 150ms) ease-out both;
  }

  .wp-card {
    position: relative;
    z-index: 10;
    width: 100%;
    max-width: 600px;
    border-radius: var(--radius-card);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    animation: wp-fade-in var(--duration-fast, 150ms) ease-out both;
  }

  /* The card surface. The kit Command primitive deliberately ships
     unpainted — the consumer owns its colors (see the comment in
     ui-kit command.svelte). */
  :global(.wp-root) {
    background: var(--color-bg-shell);
    color: var(--color-fg-shell);
  }

  :global(.wp-list) {
    max-height: 400px;
    overflow-y: auto;
    scrollbar-width: none;
    transition: opacity 80ms ease;
  }

  :global(.wp-list::-webkit-scrollbar) {
    display: none;
  }

  /* Row anatomy and the inline-preview card live in
     waypointer/WaypointerResult.svelte and
     waypointer/WaypointerInlinePreview.svelte. */

  /* Tighter than the empty-state it shares a class with: this one sits above the
     list rather than filling it, and 1.5rem of padding for one sentence pushed
     the results down the card. */
  .wp-action-error {
    padding: 0.5rem 1rem;
  }

  .wp-empty {
    padding: 1.5rem 1rem;
    text-align: center;
    font-size: var(--text-sm);
    /* 50%, not 45%. At 45 this composites to #767676 on the launcher's #0a0a0a,
       which is 4.35:1 where AA wants 4.5 - and the sentence it carries is the
       only thing on the surface when a search finds nothing or an action is
       refused. Muted is the intent; unreadable is not. */
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
  }

  .wp-footer {
    display: flex;
    gap: 14px;
    padding: 6px 12px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    font-size: var(--text-2xs);
    /* 50, measured on the launcher's own ground (#0a0a0a, darker than an app
       window): 45% is #767676 there for 4.36, just under the floor. The hints
       still sit well behind the results at 130 grey. */
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
  }

  /* Suppress pointer hover selection while navigating with keyboard. */
  :global(.wp-kb-active [data-slot="command-item"]) {
    pointer-events: none;
  }

  /* Small inline action button (used by clipboard entries for per-row
     delete). Sits at the right edge of the command item; clicks don't
     bubble to the item's onSelect. */
  .wp-inline-btn {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    background: transparent;
    border: 0;
    border-radius: var(--radius-chip);
    color: var(--color-fg-shell);
    opacity: 0.35;
    transition: background 80ms ease, opacity 80ms ease;
  }
  .wp-inline-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    opacity: 0.9;
  }

  /* The last frame drops the transform rather than setting it to the identity.
     `animation-fill-mode: both` keeps the final keyframe applied forever, and a
     transform of any value - `scale(1) translateY(0)` included - pins the element
     to its own compositing layer. The card is the element that resizes on every
     keystroke, so that looked like PR-20's cause.

     It is NOT: built an image with this change and the stale strips came back
     identical. Kept anyway, because pinning a compositing layer for an identity
     transform buys nothing, and recorded here so the next person reads a measured
     negative instead of spending a build on the same idea. */
  @keyframes wp-fade-in {
    from { opacity: 0; transform: scale(0.98) translateY(-4px); }
    to { opacity: 1; transform: none; }
  }

  @keyframes wp-backdrop-fade {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
