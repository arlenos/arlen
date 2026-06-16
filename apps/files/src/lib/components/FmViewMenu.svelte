<script lang="ts">
  /// The "View" dropdown in the headerbar: how the focused pane renders
  /// (List/Grid/Columns), the dual-pane split, hidden files and the info
  /// panel — one collapsed control in place of a five-button cluster. The
  /// trigger shows the active layout icon so the current mode reads at a
  /// glance. Under the shell the topbar View menu mirrors this depth, but
  /// the dropdown is the standalone-safe home and the quick reach.
  import {
    ChevronDown,
    Columns3,
    Eye,
    Info,
    LayoutGrid,
    List,
    SquareSplitHorizontal,
  } from "@lucide/svelte";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import type { ViewMode } from "@arlen/ui-kit/components/browser";
  import { focusedController, splitView, toggleSplit } from "$lib/stores/panes";
  import { infoOpen } from "$lib/stores/ui";

  const VIEW_OPTIONS: { value: ViewMode; label: string; icon: typeof List }[] = [
    { value: "list", label: "List", icon: List },
    { value: "grid", label: "Grid", icon: LayoutGrid },
    { value: "miller", label: "Columns", icon: Columns3 },
  ];

  // Live mirrors of the focused controller's stores.
  let mode = $state<ViewMode>("list");
  let hidden = $state(false);
  $effect(() => {
    const c = $focusedController;
    if (!c) return;
    const u1 = c.viewMode.subscribe((v) => (mode = v));
    const u2 = c.showHidden.subscribe((v) => (hidden = v));
    return () => {
      u1();
      u2();
    };
  });

  const activeIcon = $derived(
    VIEW_OPTIONS.find((o) => o.value === mode)?.icon ?? List,
  );
</script>

{#if $focusedController}
  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        {@const Icon = activeIcon}
        <button class="view-trigger" aria-label="View options" {...props}>
          <Icon size={15} strokeWidth={1.75} />
          <ChevronDown size={12} strokeWidth={2} class="chev" />
        </button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content align="end" sideOffset={4} class="fm-menu">
      {#each VIEW_OPTIONS as opt (opt.value)}
        {@const Icon = opt.icon}
        <DropdownMenu.CheckboxItem
          checked={mode === opt.value}
          closeOnSelect={false}
          onSelect={() => $focusedController?.viewMode.set(opt.value)}
        >
          <Icon />
          {opt.label}
        </DropdownMenu.CheckboxItem>
      {/each}
      <DropdownMenu.Separator />
      <DropdownMenu.CheckboxItem
        checked={$splitView}
        closeOnSelect={false}
        onSelect={() => toggleSplit()}
      >
        <SquareSplitHorizontal />
        Split panes
      </DropdownMenu.CheckboxItem>
      <DropdownMenu.CheckboxItem
        checked={hidden}
        closeOnSelect={false}
        onSelect={() => $focusedController?.setShowHidden(!hidden)}
      >
        <Eye />
        Show hidden files
      </DropdownMenu.CheckboxItem>
      <DropdownMenu.CheckboxItem
        checked={$infoOpen}
        closeOnSelect={false}
        onSelect={() => infoOpen.update((v) => !v)}
      >
        <Info />
        Show info panel
      </DropdownMenu.CheckboxItem>
    </DropdownMenu.Content>
  </DropdownMenu.Root>
{/if}

<style>
  .view-trigger {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: var(--height-control, 28px);
    padding: 0 6px;
    border: none;
    background: transparent;
    border-radius: var(--radius-button);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .view-trigger:hover,
  .view-trigger[aria-expanded="true"] {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .view-trigger :global(.chev) {
    opacity: 0.7;
  }
</style>
