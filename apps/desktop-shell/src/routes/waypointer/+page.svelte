<script lang="ts">
  import { onMount, tick } from "svelte";
  import { waypointerVisible, initWaypointerListeners, closeWaypointer } from "$lib/stores/waypointer.js";
  import { resolveAppIcon } from "$lib/stores/appIcons.js";
  import { invoke as tauriInvoke } from "@tauri-apps/api/core";
  import {
    Command, CommandInput, CommandList, CommandEmpty,
    CommandGroup, CommandItem, CommandSeparator, CommandShortcut,
  } from "$lib/components/ui/command/index.js";
  import { Search, AppWindow } from "lucide-svelte";

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

    setTimeout(() => {
      query = "";
      commandValue = "";
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
      <CommandList
        class="wp-list {kbActive ? 'wp-kb-active' : ''}"
        bind:ref={listRef}
      >
        <CommandEmpty>
          {#if loading}
            Loading apps...
          {:else}
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
