<script lang="ts">
  import { onMount, tick } from "svelte";
  import { writable } from "svelte/store";
  import { waypointerVisible, initWaypointerListeners, closeWaypointer } from "$lib/stores/waypointer.js";
  import { resolveAppIcon } from "$lib/stores/appIcons.js";
  import { invoke as tauriInvoke } from "@tauri-apps/api/core";
  import {
    Command, CommandInput, CommandList, CommandEmpty,
    CommandGroup, CommandItem, CommandSeparator, CommandShortcut,
  } from "$lib/components/ui/command/index.js";
  import { Search, AppWindow, Calculator, ArrowRightLeft } from "lucide-svelte";

  interface AppEntry {
    name: string;
    exec: string;
    icon_name: string;
    description: string;
    categories: string[];
  }

  let query = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  let listRef = $state<HTMLElement | null>(null);
  let commandValue = $state("");

  let apps = $state<AppEntry[]>([]);
  let icons = $state<Record<string, string | null>>({});
  let loading = $state(true);

  // Fetch apps once on mount.
  onMount(async () => {
    initWaypointerListeners();
    try {
      apps = await tauriInvoke<AppEntry[]>("get_apps");
      loading = false;
      // Resolve icons in the background.
      for (const app of apps) {
        if (app.icon_name && !(app.icon_name in icons)) {
          resolveAppIcon(app.icon_name).then((url) => {
            if (url) icons = { ...icons, [app.icon_name]: url };
          });
        }
      }
    } catch {
      loading = false;
    }
  });

  function open() {
    query = "";
    commandValue = "";
    inlineResult.set(null);

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
      let r: WaypointerResult | null = null;
      inlineResult.subscribe((v) => { r = v; })();
      if (r) {
        e.preventDefault();
        copyResult(r);
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

  // Debounced evaluation on query change.
  onMount(() => {
    const unsub2 = (() => {
      let prev = "";
      return setInterval(() => {
        const q = inputRef?.value ?? query;
        if (q === prev) return;
        console.error("[wp-eval] query changed:", JSON.stringify(q));
        prev = q;
        if (q.trim().length < 2) {
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
              if (wrap) wrap.style.display = "";
              // Hide the app list if it has no visible (non-hidden) items.
              if (list) {
                const hasVisible = list.querySelector("[data-slot='command-item']:not([hidden])");
                if (!hasVisible) {
                  list.style.display = "none";
                  if (wrap) wrap.style.paddingBottom = "8px";
                } else {
                  list.style.display = "";
                  if (wrap) wrap.style.paddingBottom = "";
                }
              }
            } else {
              if (wrap) { wrap.style.display = "none"; wrap.style.paddingBottom = ""; }
              if (list) list.style.display = "";
            }
          })
          .catch(() => {
            inlineResult.set(null);
            const wrap = document.getElementById("wp-inline-wrap");
            const list = document.querySelector("[data-slot='command-list']") as HTMLElement | null;
            if (wrap) wrap.style.display = "none";
            if (list) list.style.display = "";
          });
      }, 150);
    })();
    return () => clearInterval(unsub2);
  });

  function copyResult(result: WaypointerResult) {
    navigator.clipboard.writeText(result.copy_value).catch(() => {});
    close();
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
    <Command class="wp-root" shouldFilter={true} bind:value={commandValue}>
      <CommandInput
        placeholder="Search apps..."
        bind:value={query}
        bind:ref={inputRef}
        autofocus
      />

      <!-- Inline result: above the scrollable list, always in DOM -->
      <div id="wp-inline-wrap" style="display: none; padding: 6px 6px 2px;">
        <div class="wp-inline-card" onclick={() => { const r = $inlineResult; if (r) copyResult(r); }}>
          <Calculator size={18} strokeWidth={1.5} />
          <span id="wp-inline-result" class="wp-inline-result"></span>
          <span class="wp-inline-hint">Copy</span>
        </div>
      </div>

      <CommandList
        class="wp-list {kbActive ? 'wp-kb-active' : ''}"
        bind:ref={listRef}
      >
        <CommandEmpty>
          {#if loading}
            Loading apps...
          {:else if !$inlineResult}
            No results found.
          {/if}
        </CommandEmpty>

        {#if !loading && apps.length > 0}
          <CommandGroup heading="Applications">
            {#each apps as app}
              <CommandItem
                value="{app.name} {app.description} {app.categories.join(' ')}"
                onSelect={() => launchApp(app)}
              >
                {#if icons[app.icon_name]}
                  <img
                    src={icons[app.icon_name]}
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
