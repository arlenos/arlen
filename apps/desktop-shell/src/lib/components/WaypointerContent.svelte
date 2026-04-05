<script lang="ts">
  import { writable } from "svelte/store";
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
    Command, CommandInput, CommandList, CommandEmpty,
    CommandGroup, CommandItem, CommandSeparator, CommandShortcut,
  } from "$lib/components/ui/command/index.js";
  import { Search, AppWindow, Calculator, ArrowRightLeft, TerminalSquare, BookOpen, Clock, Globe, Link } from "lucide-svelte";

  let query = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  let listRef = $state<HTMLElement | null>(null);
  let commandValue = $state("");

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
    initWaypointerListeners();
    fetchAllApps()
      .then((apps) => { allApps = apps; searchResults.set(apps); })
      .catch(() => {});
  });

  function doSearch(q: string) {
    if (!q.trim()) {
      searchResults.set(allApps);
      return;
    }
    searchApps(q)
      .then((r) => { searchResults.set(r); })
      .catch(() => { searchResults.set([]); });
  }

  function debouncedSearch(q: string) {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      doSearch(q);
      updateWindowResults(q);
    }, 100);
  }

  function open() {
    query = "";
    commandValue = "";
    inlineResult.set(null);
    clearWindowResults();
    searchResults.set(allApps);
    // Scroll list to top. Focus is handled by Rust eval() immediately
    // after show() -- no setTimeout needed here.
    if (listRef) listRef.scrollTop = 0;
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
  type SpecialMode = "shell" | "man" | "url" | "search" | null;
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

  /// Checks if a string looks like a URL.
  function looksLikeUrl(s: string): boolean {
    if (/^https?:\/\//i.test(s)) return true;
    // domain.tld pattern (at least one dot, TLD 2-10 chars, no spaces)
    if (/^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z]{2,10})+([\/\?#].*)?$/i.test(s)) return true;
    return false;
  }

  // Poll for query changes and trigger search + evaluation.
  let _pollInterval: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (_pollInterval) return;
    let prev = "";
    _pollInterval = setInterval(() => {
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

        // Normal mode: clear special state.
        specialMode.set(null);
        specialArg.set("");
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
        evaluateInput(q)
          .then((r) => {
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
    return () => { if (_pollInterval) { clearInterval(_pollInterval); _pollInterval = null; } };
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

  /// Looks up the app icon (base64 data URL) for a window's app_id.
  function windowIcon(appId: string): string | null {
    const lower = appId.toLowerCase();
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
        <CommandEmpty>
          {#if !$inlineResult}
            No results found.
          {/if}
        </CommandEmpty>

        {#if $windowResults.length > 0}
          <CommandGroup heading="Windows">
            {#each $windowResults as win}
              {@const icon = windowIcon(win.app_id)}
              <CommandItem
                value={win.title}
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

        {#if $searchResults.length > 0}
          <CommandGroup heading="Applications">
            {#each $searchResults as app}
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
    background: rgba(0, 0, 0, 0.4);
    overflow: hidden;
    animation: wp-backdrop-fade 150ms ease-out both;
  }

  .wp-card {
    position: relative;
    z-index: 10;
    width: 100%;
    max-width: 600px;
    border-radius: 12px;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell, #fafafa) 15%, transparent);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    animation: wp-fade-in 150ms ease-out both;
  }

  :global(.wp-root) {
    background: var(--color-bg-shell, #09090b) !important;
    color: var(--color-fg-shell, #fafafa) !important;
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
    border-radius: 6px;
    background: color-mix(in srgb, var(--color-fg-shell, #fafafa) 8%, transparent);
    color: var(--color-fg-shell, #fafafa);
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
    background: var(--color-bg-shell, #09090b);
    border-radius: 2px;
    color: var(--color-fg-shell, #fafafa);
    opacity: 0.7;
  }

  .wp-app-icon {
    width: 20px;
    height: 20px;
    border-radius: 3px;
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
