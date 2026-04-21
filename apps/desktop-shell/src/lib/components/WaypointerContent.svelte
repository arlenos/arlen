<script lang="ts">
  import { writable } from "$lib/stores/svelteRe.js";
  import { waypointerVisible, initWaypointerListeners, closeWaypointer } from "$lib/stores/waypointer.js";
  import {
    fetchAllApps, searchApps, launchApp as launchAppCmd, evaluateInput, executeShellCommand,
    openUrl, webSearch, type AppEntry, type WaypointerResult,
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
    Command, CommandInput, CommandList, CommandEmpty,
    CommandGroup, CommandItem, CommandSeparator, CommandShortcut,
  } from "$lib/components/ui/command/index.js";
  import { Search, AppWindow, Calculator, ArrowRightLeft, TerminalSquare, BookOpen, Clock, Globe, Link, Skull, FolderKanban, X, Settings2 } from "lucide-svelte";
  import { activeProjects, activateFocus, deactivateFocus, isFocused, focusState, loadProjects } from "$lib/stores/projects.js";
  import {
    settingsResults, searchSettings, clearSettingsResults,
    reloadSettingsIndex, openSettingsDeepLink,
  } from "$lib/stores/settingsSearch.js";
  import WaypointerSettingInline from "./WaypointerSettingInline.svelte";

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
    console.timeLog("wp-open", "stores cleared");
    // Show max 8 suggested apps on empty query instead of ALL apps.
    // Rendering 100-200 CommandItems with base64 icons was the main
    // bottleneck — each open created hundreds of DOM nodes.
    searchResults.set(allApps.slice(0, 8));
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
      let r: WaypointerResult | null = null;
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

  interface WaypointerResult {
    result_type: string;
    display: string;
    copy_value: string;
  }

  const inlineResult = writable<WaypointerResult | null>(null);

  // Special mode: '>' = shell command, '#' = man page.
  type SpecialMode = "shell" | "man" | "url" | "search" | "kill" | "unicode" | "projects" | null;
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
          if (hint) hint.textContent = "Open man page";
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
          if (el) el.textContent = searchQuery ? `Search: ${searchQuery}` : "Type a search query...";
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
          if (hint) hint.textContent = "Open URL";
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
              if (hint) hint.textContent = r.result_type === "error" ? "Open Calculator" : "Copy";
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

  function handleInlineAction(result: WaypointerResult) {
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
    launchAppCmd(app.exec);
    close();
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
        placeholder="Search apps..."
        bind:value={query}
        bind:ref={inputRef}
        autofocus
      />

      <!-- Inline result: above the scrollable list, always in DOM -->
      <div id="wp-inline-wrap" style="display: none; padding: 6px 6px 2px;">
        <div class="wp-inline-card" onclick={() => {
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
        }}>
          <span id="wp-inline-icon" class="wp-inline-icon">
            {#if $specialMode === "shell"}
              <TerminalSquare size={18} strokeWidth={1.5} />
            {:else if $specialMode === "man"}
              <BookOpen size={18} strokeWidth={1.5} />
            {:else if $specialMode === "url"}
              <Globe size={18} strokeWidth={1.5} />
            {:else if $specialMode === "search"}
              <Search size={18} strokeWidth={1.5} />
            {:else if $specialMode === "kill"}
              <Skull size={18} strokeWidth={1.5} />
            {:else if $inlineResult?.result_type === "datetime"}
              <Clock size={18} strokeWidth={1.5} />
            {:else if $inlineResult?.result_type === "unit"}
              <ArrowRightLeft size={18} strokeWidth={1.5} />
            {:else}
              <Calculator size={18} strokeWidth={1.5} />
            {/if}
          </span>
          <span id="wp-inline-result" class="wp-inline-result"></span>
          <span id="wp-inline-hint" class="wp-inline-hint">Copy</span>
        </div>
      </div>

      <CommandList
        class="wp-list {kbActive ? 'wp-kb-active' : ''}"
        bind:ref={listRef}
      >
        <!-- CommandEmpty is unusable with shouldFilter={false} because
             cmdk always reports 0 internal matches. Use our own check
             across all provider stores instead. -->
        {#if !$inlineResult && $searchResults.length === 0 && $windowResults.length === 0 && $settingsResults.length === 0 && $unicodeResults.length === 0 && filteredProjects.length === 0 && query.trim().length > 0}
          <div class="wp-empty">No results found.</div>
        {/if}

        {#if $windowResults.length > 0}
          <CommandGroup heading="Windows">
            {#each $windowResults as win (win.id)}
              {@const icon = appIconFor(win.app_id)}
              <CommandItem
                value={`window-${win.id}`}
                onSelect={() => switchToWindow(win)}
              >
                {#if icon}
                  <span class="wp-win-icon-wrap">
                    <img src={icon} alt="" class="wp-app-icon" />
                    <span class="wp-win-badge">
                      <AppWindow size={8} strokeWidth={2} />
                    </span>
                  </span>
                {:else}
                  <AppWindow size={16} strokeWidth={1.5} />
                {/if}
                <div class="wp-app-info">
                  <span class="wp-app-name">{win.title}</span>
                  <span class="wp-app-desc">{win.app_id}</span>
                </div>
              </CommandItem>
            {/each}
          </CommandGroup>
          {#if $searchResults.length > 0}
            <CommandSeparator />
          {/if}
        {/if}

        {#if $processResults.length > 0}
          <CommandGroup heading="Processes (Enter: SIGTERM / Shift+Enter: SIGKILL)">
            {#each $processResults as proc (proc.pid)}
              {@const procIcon = appIconFor(proc.name)}
              <CommandItem
                value={`proc-${proc.pid}`}
                onSelect={() => killProcessAction(proc, false)}
              >
                {#if procIcon}
                  <span class="wp-win-icon-wrap">
                    <img src={procIcon} alt="" class="wp-app-icon" />
                    <span class="wp-win-badge wp-kill-badge">
                      <Skull size={8} strokeWidth={2} />
                    </span>
                  </span>
                {:else}
                  <Skull size={16} strokeWidth={1.5} />
                {/if}
                <div class="wp-app-info">
                  <span class="wp-app-name">{proc.name}</span>
                  <span class="wp-app-desc">PID: {proc.pid} · {formatBytes(proc.memory_bytes)}</span>
                </div>
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
                <span class="wp-unicode-char">{uc.char_str}</span>
                <div class="wp-app-info">
                  <span class="wp-app-name">{uc.name}</span>
                  <span class="wp-app-desc">{uc.codepoint_hex}</span>
                </div>
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}

        {#if filteredProjects.length > 0 || $isFocused}
          <CommandGroup heading="Projects">
            {#if $isFocused}
              <CommandItem value="focus-exit" onSelect={() => { deactivateFocus(); close(); }}>
                <X size={16} strokeWidth={1.5} class="shrink-0 opacity-60" />
                <div class="wp-app-info">
                  <span class="wp-app-name">Exit Focus: {$focusState.projectName}</span>
                </div>
              </CommandItem>
            {/if}
            {#each filteredProjects as project (project.id)}
              <CommandItem value={`focus-${project.id}`} onSelect={() => { activateFocus(project); close(); }}>
                <FolderKanban size={16} strokeWidth={1.5} class="shrink-0 opacity-60" />
                <div class="wp-app-info">
                  <span class="wp-app-name">{project.name}</span>
                  <span class="wp-app-desc">{project.rootPath}</span>
                </div>
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
                <Settings2 size={16} strokeWidth={1.5} class="shrink-0 opacity-60" />
                <div class="wp-app-info" style="flex: 1; min-width: 0;">
                  <span class="wp-app-name">{sr.setting.title}</span>
                  <span class="wp-app-desc">{sr.setting.section}</span>
                </div>
                {#if sr.setting.inlineAction}
                  <WaypointerSettingInline
                    action={sr.setting.inlineAction}
                    {query}
                  />
                {/if}
              </CommandItem>
            {/each}
          </CommandGroup>
          <CommandSeparator />
        {/if}

        {#if $searchResults.length > 0}
          <CommandGroup heading="Applications">
            {#each $searchResults as app, i (app.name + i)}
              <CommandItem
                value={app.name}
                onSelect={() => launchAppAndClose(app)}
              >
                {#if app.icon_data}
                  <img
                    src={app.icon_data}
                    alt=""
                    class="wp-app-icon"
                  />
                {:else}
                  <AppWindow size={16} strokeWidth={1.5} class="wp-fallback-icon" />
                {/if}
                <div class="wp-app-info">
                  <span class="wp-app-name">{app.name}</span>
                  {#if app.description}
                    <span class="wp-app-desc">{app.description}</span>
                  {/if}
                </div>
              </CommandItem>
            {/each}
          </CommandGroup>
        {/if}
      </CommandList>
    </Command>
  </div>
</div>

<style>
  :global(html), :global(body) {
    background: transparent !important;
    overflow: hidden !important;
    height: 100% !important;
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
    animation: wp-backdrop-fade 150ms ease-out both;
  }

  .wp-card {
    position: relative;
    z-index: 10;
    width: 100%;
    max-width: 600px;
    border-radius: var(--radius-lg);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    animation: wp-fade-in 150ms ease-out both;
  }

  :global(.wp-root) {
    background: var(--color-bg-shell) !important;
    color: var(--color-fg-shell) !important;
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

  .wp-inline-card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    color: var(--color-fg-shell);
  }

  .wp-inline-result {
    font-size: 1.125rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .wp-inline-hint {
    margin-left: auto;
    font-size: 0.6875rem;
    opacity: 0.35;
  }

  .wp-win-icon-wrap {
    position: relative;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
  }

  .wp-win-badge {
    position: absolute;
    bottom: -3px;
    right: -3px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    background: var(--color-bg-shell);
    border-radius: var(--radius-sm);
    color: var(--color-fg-shell);
    opacity: 0.7;
  }

  :global(.wp-kill-badge) {
    color: var(--color-error);
    opacity: 0.9;
  }

  .wp-unicode-char {
    font-size: 1.25rem;
    line-height: 1;
    width: 24px;
    text-align: center;
    flex-shrink: 0;
  }

  .wp-app-icon {
    width: 20px;
    height: 20px;
    border-radius: var(--radius-sm);
    object-fit: contain;
    flex-shrink: 0;
  }

  :global(.wp-fallback-icon) {
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    opacity: 0.4;
  }

  .wp-app-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .wp-app-name {
    font-size: 0.8125rem;
    line-height: 1.3;
  }

  .wp-app-desc {
    font-size: 0.6875rem;
    line-height: 1.3;
    opacity: 0.45;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .wp-empty {
    padding: 1.5rem 1rem;
    text-align: center;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);
  }

  /* Suppress pointer hover selection while navigating with keyboard. */
  :global(.wp-kb-active [data-slot="command-item"]) {
    pointer-events: none;
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
