<script lang="ts">
  import { onMount, tick } from "svelte";
  import { waypointerVisible, initWaypointerListeners, closeWaypointer } from "$lib/stores/waypointer.js";
  import {
    Command, CommandInput, CommandList, CommandEmpty,
    CommandGroup, CommandItem, CommandSeparator, CommandShortcut,
  } from "$lib/components/ui/command/index.js";
  import { Terminal, FolderOpen, Settings, Calculator, Globe, FileText } from "lucide-svelte";

  let query = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  let listRef = $state<HTMLElement | null>(null);
  let commandValue = $state("");
  function open() {
    query = "";
    commandValue = "";

    // Reset after window is visible. The 150ms fade-in animation
    // hides any flash of stale content.
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
    initWaypointerListeners();
    return unsub;
  });

  function close() {
    closeWaypointer();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  function selectItem(label: string) {
    console.info("waypointer: selected", label);
    close();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="wp-backdrop" onclick={close}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="wp-card shell-surface" onclick={(e) => e.stopPropagation()}>
    <Command class="wp-root" shouldFilter={true} bind:value={commandValue}>
      <CommandInput
        placeholder="Type a command or search..."
        bind:value={query}
        bind:ref={inputRef}
        autofocus
      />
      <CommandList class="wp-list" bind:ref={listRef}>
        <CommandEmpty>No results found.</CommandEmpty>

        <CommandGroup heading="Applications">
          <CommandItem value="terminal" onSelect={() => selectItem("Terminal")}>
            <Terminal size={16} strokeWidth={1.5} />
            <span>Terminal</span>
            <CommandShortcut>Ctrl+Alt+T</CommandShortcut>
          </CommandItem>
          <CommandItem value="files" onSelect={() => selectItem("Files")}>
            <FolderOpen size={16} strokeWidth={1.5} />
            <span>Files</span>
          </CommandItem>
          <CommandItem value="settings" onSelect={() => selectItem("Settings")}>
            <Settings size={16} strokeWidth={1.5} />
            <span>Settings</span>
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        <CommandGroup heading="Tools">
          <CommandItem value="calculator" onSelect={() => selectItem("Calculator")}>
            <Calculator size={16} strokeWidth={1.5} />
            <span>Calculator</span>
          </CommandItem>
          <CommandItem value="browser" onSelect={() => selectItem("Web Browser")}>
            <Globe size={16} strokeWidth={1.5} />
            <span>Web Browser</span>
          </CommandItem>
          <CommandItem value="text-editor" onSelect={() => selectItem("Text Editor")}>
            <FileText size={16} strokeWidth={1.5} />
            <span>Text Editor</span>
          </CommandItem>
        </CommandGroup>
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

  @keyframes wp-fade-in {
    from { opacity: 0; transform: scale(0.98) translateY(-4px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  @keyframes wp-backdrop-fade {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  :global(.wp-root) {
    background: var(--color-bg-shell, #09090b) !important;
    color: var(--color-fg-shell, #fafafa) !important;
  }

  :global(.wp-list) {
    max-height: 400px;
  }
</style>
