<script lang="ts">
  import { onMount, tick } from "svelte";
  import { writable } from "svelte/store";
  import { waypointerVisible, initWaypointerListeners, closeWaypointer } from "$lib/stores/waypointer.js";
  import { invoke as tauriInvoke } from "@tauri-apps/api/core";
  import {
    Command, CommandInput, CommandList, CommandEmpty,
    CommandGroup, CommandItem, CommandSeparator, CommandShortcut,
  } from "$lib/components/ui/command/index.js";
  import { Search, AppWindow, Calculator, ArrowRightLeft, TerminalSquare, BookOpen } from "lucide-svelte";

  interface AppEntry {
    name: string;
    exec: string;
    icon_name: string;
    icon_data: string | null;
    description: string;
    categories: string[];
  }

  let query = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  let listRef = $state<HTMLElement | null>(null);
  let commandValue = $state("");

  // App search results from Rust (max 20, pre-filtered, icons included).
  const searchResults = writable<AppEntry[]>([]);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  // Full app list cached for the calculator fallback.
  let allApps: AppEntry[] = [];

  onMount(async () => {
    initWaypointerListeners();
    try {
      allApps = await tauriInvoke<AppEntry[]>("get_apps");
    } catch { /* ignore */ }
    searchResults.set(allApps);
  });

  function doSearch(q: string) {
    if (!q.trim()) {
      searchResults.set(allApps);
      return;
    }
    tauriInvoke<AppEntry[]>("search_apps", { query: q })
      .then((r) => { searchResults.set(r); })
      .catch(() => { searchResults.set([]); });
  }

  function debouncedSearch(q: string) {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => doSearch(q), 100);
  }

  function open() {
    query = "";
    commandValue = "";
    inlineResult.set(null);
    searchResults.set(allApps);

    setTimeout(() => {
      query = "";
      commandValue = "";
      inlineResult.set(null);
      if (inputRef) {
        inputRef.value = "";
        inputRef.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if (listRef) listRef.scrollTop = 0;
      inputRef?.focus();
      setTimeout(() => inputRef?.focus(), 100);
    }, 200);
  }

  onMount(() => {
    const unsub = waypointerVisible.subscribe((visible) => {
      if (visible) open();
    });
    return unsub;
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
        console.error(`[wp] shell: cmd="${arg}" shift=${e.shiftKey} inTerminal=${e.shiftKey}`);
        runShellCommand(arg, e.shiftKey);
        return;
      }
      if (mode === "man" && arg) {
        e.preventDefault();
        openManPage(arg);
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
  type SpecialMode = "shell" | "man" | null;
  const specialMode = writable<SpecialMode>(null);
  const specialArg = writable<string>("");

  function runShellCommand(cmd: string, inTerminal: boolean) {
    console.error(`[wp] runShellCommand: cmd="${cmd}" inTerminal=${inTerminal}`);
    tauriInvoke("execute_shell_command", { command: cmd, inTerminal })
      .catch((err) => console.error("[wp] execute_shell_command error:", err));
    close();
  }

  function openManPage(topic: string) {
    tauriInvoke("execute_shell_command", { command: `man ${topic}`, inTerminal: true });
    close();
  }

  // Poll for query changes and trigger search + evaluation.
  onMount(() => {
    const unsub2 = (() => {
      let prev = "";
      return setInterval(() => {
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
          if (wrap) wrap.style.display = cmd ? "" : "none";
          if (el) el.textContent = cmd || "Type a command...";
          if (hint) hint.textContent = "Enter: Run / Shift+Enter: Terminal";
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
          if (wrap) wrap.style.display = topic ? "" : "none";
          if (el) el.textContent = topic ? `man ${topic}` : "Type a topic...";
          if (hint) hint.textContent = "Open man page";
          return;
        }

        // Normal mode: clear special state.
        specialMode.set(null);
        specialArg.set("");

        // Search apps in Rust.
        debouncedSearch(q);
        // Evaluate math/units.
        if (trimmed.length < 2) {
          inlineResult.set(null);
          return;
        }
        tauriInvoke<WaypointerResult | null>("evaluate_waypointer_input", { input: q })
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
            } else {
              if (wrap) { wrap.style.display = "none"; }
            }
          })
          .catch(() => {
            inlineResult.set(null);
            const wrap = document.getElementById("wp-inline-wrap");
            if (wrap) wrap.style.display = "none";
          });
      }, 150);
    })();
    return () => clearInterval(unsub2);
  });

  function handleInlineAction(result: WaypointerResult) {
    if (result.result_type === "error") {
      // Launch a calculator app from the index.
      const calc = allApps.find((a) =>
        a.name.toLowerCase().includes("calculator") ||
        a.name.toLowerCase().includes("rechner")
      );
      if (calc) {
        tauriInvoke("launch_app", { exec: calc.exec });
      }
      close();
    } else {
      navigator.clipboard.writeText(result.copy_value).catch(() => {});
      close();
    }
  }

  function launchApp(app: AppEntry) {
    tauriInvoke("launch_app", { exec: app.exec });
    close();
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
          const r = $inlineResult;
          if (r) handleInlineAction(r);
        }}>
          <span id="wp-inline-icon" class="wp-inline-icon">
            {#if $specialMode === "shell"}
              <TerminalSquare size={18} strokeWidth={1.5} />
            {:else if $specialMode === "man"}
              <BookOpen size={18} strokeWidth={1.5} />
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

        {#if $searchResults.length > 0}
          <CommandGroup heading="Applications">
            {#each $searchResults as app}
              <CommandItem
                value={app.name}
                onSelect={() => launchApp(app)}
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
