<script lang="ts">
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
    unicodeResults, updateUnicodeResults, clearUnicodeResults,
    type UnicodeChar,
  } from "$lib/stores/waypointerUnicode.js";
  import {
    Command, CommandInput, CommandList,
    CommandGroup, CommandItem, CommandSeparator,
  } from "@arlen/ui-kit/components/ui/command/index.js";
  import { AppWindow, BookOpen, Globe, Skull, FolderKanban, X, Settings2 } from "lucide-svelte";
  import { activeProjects, activateFocus, deactivateFocus, isFocused, focusState, loadProjects } from "$lib/stores/projects.js";
  import {
    settingsResults, searchSettings, clearSettingsResults,
    reloadSettingsIndex, openSettingsDeepLink,
  } from "$lib/stores/settingsSearch.js";
  import WaypointerSettingInline from "./WaypointerSettingInline.svelte";
  import WaypointerResult from "./waypointer/WaypointerResult.svelte";
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
      .catch(() => { searchResults.set([]); });
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
        .catch(() => {});
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
        .catch(() => {});
      // File-search plugin: same bridge, separate section.
      const t3 = performance.now();
      updateFileResults(q)
        .then(() => {
          console.log(
            `[wp-search] files: ${(performance.now() - t3).toFixed(1)}ms`,
          );
        })
        .catch(() => {});
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
        .catch(() => {});
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
        .catch(() => {});
      // TEMPORARILY DISABLED: same bisection as the worker-pool
      // init effect above. If a hidden iframe host is the layout
      // regressor, even silently calling searchModules with no
      // workers shouldn't matter, but cutting the call avoids any
      // listener side-effects too.
      void searchModules;
      void moduleResults;
      // const t6 = performance.now();
      // searchModules(q)
      //   .then((results) => {
      //     moduleResults.set(results);
      //   })
      //   .catch(() => {
      //     moduleResults.set([]);
      //   });
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
    clearUnicodeResults();
    clearSettingsResults();
    clearPowerResults();
    clearQuickActionResults();
    clearFileResults();
    clearClipboardResults();
    clearDictResults();
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
    closeWaypointer();
  }

  let kbActive = $state(false);
  let lastMouse = { x: 0, y: 0 };

  function handleKeydown(e: KeyboardEvent) {
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

  const specialMode = writable<SpecialMode>(null);
  const specialArg = writable<string>("");

  function runShellCommand(cmd: string, inTerminal: boolean) {
    executeShellCommand(cmd, inTerminal);
    close();
  }

  function openManPage(topic: string) {
    executeShellCommand(`man ${topic}`, true);
    close();
  }

  function openUrlAction(url: string) {
    openUrl(url);
    close();
  }

  function webSearchAction(query: string) {
    webSearch(query);
    close();
  }

  function copyUnicodeChar(uc: UnicodeChar) {
    navigator.clipboard.writeText(uc.char_str).catch(() => {});
    close();
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
    killProcess(proc.pid, force).catch(() => {});
    close();
  }

  /// Checks if a string looks like a URL.
  function looksLikeUrl(s: string): boolean {
    if (/^https?:\/\//i.test(s)) return true;
    // domain.tld pattern (at least one dot, TLD 2-10 chars, no spaces)
    if (/^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z]{2,10})+([\/\?#].*)?$/i.test(s)) return true;
    return false;
  }

  // PERMANENTLY OFF until the Tier 2 worker pool can be initialised
  // without taking over the Waypointer's flex layout. The Waypointer
  // window is a layer-shell overlay anchored to all four edges, and
  // calling `installModuleListener()` / `refreshModuleWorkers()` at
  // mount time was making the wp-card stretch to fill the window.
  // The store's body-level host insertion alone shouldn't matter —
  // it's always positioned off-screen — so the regression is
  // probably the listener install racing with the Tauri webview
  // first paint. Re-enable once we wire the worker pool from a
  // dedicated route or after the Waypointer is actually shown.
  void installModuleListener;
  void refreshModuleWorkers;

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
        const trimmed = q.trim();

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
          if (el) el.textContent = cmd || "Type a command...";
          if (hint) hint.textContent = "Enter: Run / Shift+Enter: Terminal";
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
          if (el) el.textContent = topic ? `man ${topic}` : "Type a topic...";
          if (hint) hint.textContent = "Open manual page";
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

        // "unicode" keyword: character search.
        if (trimmed.toLowerCase().startsWith("unicode")) {
          const filter = trimmed.slice(7).trim();
          specialMode.set("unicode");
          specialArg.set(filter);
          searchResults.set([]);
          inlineResult.set(null);
          if (filter) {
            updateUnicodeResults(filter);
          } else {
            clearUnicodeResults();
          }
          const wrap = document.getElementById("wp-inline-wrap");
          if (wrap) wrap.style.display = "none";
          const listUni = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
          if (listUni) listUni.style.display = "";
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
          clearUnicodeResults();
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
        clearUnicodeResults();
        // Restore list visibility.
        const listEl = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
        if (listEl) listEl.style.display = "";

        // Search apps in Rust.
        debouncedSearch(q);
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
        launchAppCmd(calc.exec);
      }
      close();
    } else {
      navigator.clipboard.writeText(result.copy_value).catch(() => {});
      close();
    }
  }

  function launchAppAndClose(app: AppEntry) {
    // Record the launch BEFORE closing: the close path may tear down
    // event listeners, and the record call is fire-and-forget so it
    // doesn't block the actual launch below.
    recordAppLaunch(app.exec);
    launchAppCmd(app.exec);
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
<div class="wp-backdrop" onclick={close}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="wp-card shell-surface" onclick={(e) => e.stopPropagation()}>
    <Command class="wp-root" shouldFilter={false} bind:value={commandValue}>
      <CommandInput
        placeholder="Search..."
        bind:value={query}
        bind:ref={inputRef}
        autofocus
      />

      <WaypointerInlinePreview
        {specialMode}
        {inlineResult}
        onActivate={activateInlinePreview}
      />

      <CommandList
        class="wp-list {kbActive ? 'wp-kb-active' : ''}"
        bind:ref={listRef}
      >
        <!-- CommandEmpty is unusable with shouldFilter={false} because
             cmdk always reports 0 internal matches. Use our own check
             across all provider stores instead. -->
        {#if !$inlineResult && $searchResults.length === 0 && $windowResults.length === 0 && $settingsResults.length === 0 && $unicodeResults.length === 0 && $powerResults.length === 0 && $quickActionResults.length === 0 && $fileResults.length === 0 && $clipboardResults.length === 0 && $dictResults.length === 0 && filteredProjects.length === 0 && $recentAppsStore.length === 0 && $recentFilesStore.length === 0 && query.trim().length > 0}
          <div class="wp-empty">No results found.</div>
        {/if}

        <!-- Power actions from the `core.power` plugin. Placed above
             the app-search group so an exact keyword like "shutdown"
             (relevance 1.0) surfaces at the top, beating any partial
             app-name match. -->
        {#if $powerResults.length > 0}
          <CommandGroup heading="System">
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
                  title={action.title}
                  description={action.description}
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
          <CommandGroup heading="Quick Actions">
            {#each $quickActionResults as qa (qa.id)}
              <CommandItem
                value={`qa-${qa.id}`}
                onSelect={() => {
                  invokeQuickAction(qa.id);
                  close();
                }}
              >
                <WaypointerResult
                  icon={Settings2}
                  title={qa.title}
                  description={qa.description}
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
          <CommandGroup heading="Recent Apps">
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
          <CommandGroup heading="Recent Files">
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
          <CommandGroup heading="Windows">
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
          <CommandGroup heading="Processes">
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
                  description={`PID ${proc.pid}, ${formatBytes(proc.memory_bytes)}`}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}

        {#if $unicodeResults.length > 0}
          <CommandGroup heading="Unicode">
            {#each $unicodeResults as uc (uc.codepoint)}
              <CommandItem
                value={`unicode-${uc.codepoint}`}
                onSelect={() => copyUnicodeChar(uc)}
              >
                <WaypointerResult
                  glyph={uc.char_str}
                  title={uc.name}
                  description={uc.codepoint_hex}
                />
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}

        {#if $searchResults.length > 0}
          <CommandGroup heading="Applications">
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
          <CommandGroup heading="Projects">
            {#if $isFocused}
              <CommandItem value="focus-exit" onSelect={() => { deactivateFocus(); close(); }}>
                <WaypointerResult
                  icon={X}
                  emphasis={60}
                  title={`Exit Focus: ${$focusState.projectName}`}
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
          <CommandGroup heading="Clipboard">
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
                      aria-label="Remove from history"
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
                  title="Clear clipboard history"
                  description="Removes the entire history"
                />
              </CommandItem>
            {/if}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $fileResults.length > 0}
          <CommandGroup heading="Files">
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
          <CommandGroup heading="Definitions">
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

        {#if $settingsResults.length > 0}
          <CommandGroup heading="Settings">
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
          <CommandGroup heading="Modules">
            {#each $moduleResults as result (result.id)}
              <CommandItem
                value={`module:${result.id}`}
                onSelect={async () => {
                  if (result.action.type === "copy") {
                    try { await navigator.clipboard.writeText(result.action.text); } catch {}
                  } else if (result.action.type === "open_url") {
                    try { await invoke("open_url", { url: result.action.url }); } catch {}
                  } else if (result.action.type === "execute") {
                    try { await invoke("execute_shell_command", { command: result.action.command }); } catch {}
                  }
                  closeWaypointer();
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
      {#if $specialMode === "kill"}
        <div class="wp-footer">
          <span>Enter quits the app</span>
          <span>Shift+Enter force-quits</span>
        </div>
      {:else if query.trim().length === 0}
        <div class="wp-footer">
          <span>&gt; command</span>
          <span># manual</span>
          <span>? web search</span>
          <span>p: projects</span>
        </div>
      {/if}
    </Command>
  </div>
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

  .wp-empty {
    padding: 1.5rem 1rem;
    text-align: center;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);
  }

  .wp-footer {
    display: flex;
    gap: 14px;
    padding: 6px 12px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);
  }

  /* Suppress pointer hover selection while navigating with keyboard. */
  :global(.wp-kb-active [data-slot="command-item"]) {
    pointer-events: none;
  }

  /* Small inline action button (used by clipboard entries for per-row
     delete). Sits at the right edge of the command item; clicks don't
     bubble to the item's onSelect. */
  .wp-inline-btn {
    margin-left: auto;
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

  @keyframes wp-fade-in {
    from { opacity: 0; transform: scale(0.98) translateY(-4px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  @keyframes wp-backdrop-fade {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
