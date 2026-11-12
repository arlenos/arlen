<script lang="ts">
  /// The tabs, living in the headerbar (two-level chrome): visible
  /// only with more than one tab open. A tab shows its folder name;
  /// the close affordance appears on hover, Ctrl+W closes the active
  /// one. The gaps around the chips stay window-drag area.
  import { X } from "lucide-svelte";
  import { get } from "svelte/store";
  import { activeTabId, closeTab, selectTab, tabs, type Tab } from "$lib/stores/tabs";
  import { t } from "$lib/i18n/messages";

  const tabLabel = (tab: Tab): string => {
    const p = get(tab.controller.path);
    const name = p.split("/").filter(Boolean).pop();
    return name ?? "/";
  };

  /// The strip scrolls once the tabs are down to their floor, and the open tab
  /// is brought into view whenever it changes or the strip is resized: with
  /// five tabs at 720 px the open one was past the edge and nothing showed
  /// which tab the pane below belonged to.
  let strip = $state<HTMLElement | null>(null);
  function showOpenTab() {
    strip?.querySelector(".ts-tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }
  $effect(() => {
    void $activeTabId;
    void $tabs.length;
    if (!strip) return;
    requestAnimationFrame(showOpenTab);
    const ro = new ResizeObserver(showOpenTab);
    ro.observe(strip);
    return () => ro.disconnect();
  });

  /// A wheel has one axis and the strip has the other.
  function wheel(e: WheelEvent) {
    if (!strip || e.deltaX !== 0 || strip.scrollWidth <= strip.clientWidth) return;
    strip.scrollLeft += e.deltaY;
    e.preventDefault();
  }

  // Paths change as tabs navigate; the labels record is rebuilt from
  // a local accumulator so the effect never reads what it writes.
  let labels = $state<Record<number, string>>({});
  $effect(() => {
    const acc: Record<number, string> = {};
    const unsubs = $tabs.map((t) =>
      t.controller.path.subscribe((p) => {
        acc[t.id] = p.split("/").filter(Boolean).pop() ?? "/";
        labels = { ...acc };
      }),
    );
    return () => unsubs.forEach((u) => u());
  });
</script>

<!-- A GROUP, NOT A TABLIST, and the reason is that a tablist may hold nothing
     but tabs. This said `role="tablist"` over `div` wrappers each holding a
     `role="tab"` button AND a close button, which axe rates CRITICAL
     (`aria-required-children`): the tablist has no tabs in the accessibility
     tree at all, so the "tab 2 of 3, selected" announcement the role promises
     was never delivered either.

     Measured, all three shapes, rather than argued (15 Sep):

       as it was ................. aria-required-children, critical
       close button inside the tab  nested-interactive, serious
       a labelled group ........... no violations

     Wrapping differently does not help - `role="presentation"` on the wrapper
     and flattening it both leave the close buttons as children the tablist may
     not have. The close control is the thing a tablist cannot express, and
     removing it to satisfy the role would take away the only pointer way to
     close a tab.

     So: a labelled group of plain buttons, with `aria-current` marking the open
     one. Same precedent as `MillerColumns` in the kit - a role that promises a
     structure and delivers none is worse than a modest one that is true. -->
{#if $tabs.length > 1}
  <div class="tab-strip" role="group" aria-label={$t("f.tab.strip")} bind:this={strip} onwheel={wheel}>
    {#each $tabs as tab (tab.id)}
      <div class="ts-tab" class:active={tab.id === $activeTabId}>
        <button
          class="ts-label"
          aria-current={tab.id === $activeTabId ? "true" : undefined}
          onclick={() => selectTab(tab.id)}
        >
          {labels[tab.id] ?? tabLabel(tab)}
        </button>
        <button
          class="ts-close"
          aria-label={$t("f.tab.closeNamed", { tab: labels[tab.id] ?? tabLabel(tab) })}
          onclick={() => closeTab(tab.id)}
        >
          <X size={12} strokeWidth={2} />
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .tab-strip {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  /* Every tab is a quiet pill: next to the breadcrumb in the merged
     headerbar, a bare inactive label would read as another crumb. */
  .ts-tab {
    display: inline-flex;
    align-items: center;
    /* An explicit floor is what lets a tab shrink at all: without one a flex
       item is as wide as its label at least, and the strip cut the last tab
       mid-letter and took its close button with it. The label ellipsizes. */
    min-width: 3.5rem;
    flex-shrink: 1;
    height: var(--height-control, 30px);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-tab:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  /* The open tab keeps its name whole; the others give way and the strip
     scrolls. */
  .ts-tab.active {
    flex-shrink: 0;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }

  .ts-label {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    max-width: 12rem;
    padding: 0 4px 0 10px;
    border: none;
    background: transparent;
    color: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ts-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 1.25rem;
    height: 1.25rem;
    margin-inline-end: 4px;
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: inherit;
    opacity: 0;
    transition: opacity var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-tab:hover .ts-close,
  .ts-tab:focus-within .ts-close,
  .ts-tab.active .ts-close {
    opacity: 1;
  }
  /* A tab that is not the open one shows no close button until it is pointed
     at, and used to keep 24 px free for it all the same. In a narrow window
     that was the room its own name needed. There the button lies over the end
     of the name instead and covers it only while it shows. */
  .ts-tab:not(.active) {
    position: relative;
  }
  .ts-tab:not(.active) .ts-label {
    padding-inline-end: 10px;
  }
  .ts-tab:not(.active) .ts-close {
    position: absolute;
    inset-inline-end: 0;
    top: 50%;
    translate: 0 -50%;
    background: color-mix(in srgb, var(--foreground) 6%, var(--background));
  }
  .ts-tab:not(.active) .ts-close:hover {
    background: color-mix(in srgb, var(--foreground) 14%, var(--background));
  }
  .ts-close:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
