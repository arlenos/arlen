<script lang="ts">
  import { page } from "$app/stores";
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
    useSidebar,
  } from "$lib/components/ui/sidebar";
  import { tick } from "svelte";
  import { PANELS, navigateTo } from "$lib/stores/navigation";
  import { search, type SearchResult } from "$lib/search/index";
  import {
    Search,
    Palette,
    Keyboard,
    Command,
    Monitor,
    Mouse,
    SquareMousePointer,
    Bell,
    Shield,
    Puzzle,
    Info,
    LayoutGrid,
    Accessibility,
    Crosshair,
    Brain,
    Zap,
  } from "lucide-svelte";

  let query = $state("");
  let activeIndex = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);
  const results = $derived<SearchResult[]>(
    query.trim().length > 0 ? search(query, 8) : [],
  );

  const sidebar = useSidebar();
  const collapsed = $derived(!sidebar.open);

  /// While collapsed the search box is a glorified expand button:
  /// no placeholder text fits in 32 px, focusing the input behind
  /// the icon would feel broken. Click expands the sidebar and
  /// transfers focus to the input on the next tick (after the
  /// width animation settles).
  async function onSearchClick(e: MouseEvent) {
    if (!collapsed) return;
    e.preventDefault();
    sidebar.toggle();
    await tick();
    // Wait for the 200ms sidebar transition before focusing — the
    // browser otherwise scrolls the half-grown input into view and
    // the cursor lands in a glitchy intermediate position.
    setTimeout(() => inputEl?.focus(), 220);
  }

  function onPick(r: SearchResult) {
    // Pass the setting's `anchor` as the scrollTarget so the
    // existing deep-link handler in +layout.svelte scrolls to the
    // DOM element with that id and adds `.setting-highlight`,
    // which pulses for 3 s. The user lands on the panel with the
    // matching control already drawing the eye.
    navigateTo(r.setting.panel, r.setting.anchor);
    query = "";
    activeIndex = 0;
  }

  function onKey(e: KeyboardEvent) {
    if (results.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      activeIndex = (activeIndex + 1) % results.length;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      activeIndex = (activeIndex - 1 + results.length) % results.length;
    } else if (e.key === "Enter") {
      e.preventDefault();
      const result = results[activeIndex];
      if (result) onPick(result);
    } else if (e.key === "Escape") {
      query = "";
      activeIndex = 0;
    }
  }

  const ICONS: Record<string, typeof Palette> = {
    Palette,
    Keyboard,
    Command,
    Monitor,
    Mouse,
    SquareMousePointer,
    Bell,
    Shield,
    Puzzle,
    Info,
    LayoutGrid,
    Accessibility,
    Crosshair,
    Brain,
    Zap,
  };

  // Group panels by section, matching the Pre-Phase-6 plan
  // (`docs/architecture/settings-app.md` section structure plus
  // four new pages).
  const SECTIONS = [
    {
      label: "System",
      panelIds: ["display", "workspaces", "notifications", "about"] as const,
    },
    {
      label: "Personal",
      panelIds: [
        "appearance",
        "quicksettings",
        "accessibility",
        "focus",
        "knowledge",
      ] as const,
    },
    {
      label: "Input",
      panelIds: [
        "keyboard",
        "shortcuts",
        "mouse",
        "touchpad",
        "system-actions",
      ] as const,
    },
    {
      label: "Apps & Modules",
      panelIds: ["extensions", "privacy"] as const,
    },
  ];

  function isActive(href: string): boolean {
    return $page.url.pathname === href;
  }
</script>

<Sidebar collapsible="icon">
  <SidebarHeader>
    <!--
      Search replaces the static "Settings · Lunaris OS" branding
      that lived here. The block was decorative and the slot is on
      every Settings page; turning it into a search bar gives users a
      direct way into any setting without scanning the sidebar list.
      In `collapsible="icon"` collapsed mode the input shrinks to the
      Search icon (group-data CSS); clicking expands the sidebar.

      Result-popup styling: explicit CSS variables (not Tailwind
      `bg-popover`/`text-foreground`) because Tailwind's token
      shorthand wasn't resolving cleanly in app-settings — the
      popup rendered white. Going through the canonical
      `--color-bg-card` / `--color-fg-app` tokens directly keeps
      the popup themed in both dark and light modes.
    -->
    <div class="settings-search-wrap">
      <Search
        size={14}
        strokeWidth={2}
        class="search-icon"
      />
      <input
        type="text"
        bind:this={inputEl}
        bind:value={query}
        onkeydown={onKey}
        onmousedown={onSearchClick}
        onfocus={() => {
          if (collapsed) sidebar.toggle();
        }}
        placeholder={collapsed ? "" : "Search settings..."}
        aria-label="Search settings"
        readonly={collapsed}
        class="search-input"
      />
      {#if results.length > 0}
        <div
          class="search-popup group-data-[collapsible=icon]:hidden"
          role="listbox"
        >
          {#each results as r, i (r.setting.title + i)}
            {@const Icon =
              ICONS[
                PANELS.find((p) => p.id === r.setting.panel)?.icon ?? "Palette"
              ] ?? Palette}
            <button
              type="button"
              class="search-result"
              class:active={i === activeIndex}
              onclick={() => onPick(r)}
              onmouseenter={() => (activeIndex = i)}
              role="option"
              aria-selected={i === activeIndex}
            >
              <span class="search-result-icon">
                <Icon size={16} strokeWidth={1.75} />
              </span>
              <span class="search-result-text">
                <span class="search-result-title">{r.setting.title}</span>
                <span class="search-result-meta">
                  {r.setting.section} · {r.setting.description}
                </span>
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </SidebarHeader>

  <SidebarContent>
    {#each SECTIONS as section (section.label)}
      <SidebarGroup>
        <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
        <SidebarMenu>
          {#each section.panelIds as id}
            {@const panel = PANELS.find((p) => p.id === id)}
            {#if panel}
              {@const Icon = ICONS[panel.icon] ?? Palette}
              <SidebarMenuItem>
                <SidebarMenuButton
                  isActive={isActive(panel.href)}
                  disabled={!panel.enabled}
                  tooltip={panel.title}
                  onclick={() => navigateTo(panel.id)}
                >
                  <Icon />
                  <span>{panel.title}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            {/if}
          {/each}
        </SidebarMenu>
      </SidebarGroup>
    {/each}
  </SidebarContent>

  <SidebarRail />
</Sidebar>

<style>
  .settings-search-wrap {
    position: relative;
    width: 100%;
  }

  :global(.settings-search-wrap .search-icon) {
    position: absolute;
    left: 10px;
    top: 50%;
    transform: translateY(-50%);
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
    pointer-events: none;
  }

  .search-input {
    width: 100%;
    height: 32px;
    padding: 4px 10px 4px 30px;
    background: color-mix(in srgb, var(--color-fg-app) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-app) 8%, transparent);
    border-radius: var(--radius-input);
    color: var(--color-fg-app);
    font-size: 0.875rem;
    outline: none;
    transition:
      background 120ms ease,
      border-color 120ms ease,
      width 120ms ease,
      padding 120ms ease;
  }

  /* In icon-collapsed mode the SidebarHeader gives us the actual
     content width via flex (the sidebar's 48px minus 1px `border-r`
     and 8px each side of `p-2` lands around 31px in practice). We
     deliberately don't pin a fixed 32px width here — that overflows
     the right divider by 1px. Instead the wrap stays `width: 100%`
     and the input shrinks padding to 0 so its rounded box matches
     whatever space the flex parent assigns it. The `:global()` is
     needed because `[data-collapsible="icon"]` lives on the outer
     Sidebar wrapper, outside Svelte's class-hash scope. */
  :global([data-collapsible="icon"]) .search-input {
    padding: 4px 0;
    cursor: pointer;
  }

  :global([data-collapsible="icon"] .settings-search-wrap .search-icon) {
    left: 50%;
    transform: translate(-50%, -50%);
  }

  .search-input:focus {
    background: color-mix(in srgb, var(--color-fg-app) 10%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 60%, transparent);
  }

  .search-input::placeholder {
    color: color-mix(in srgb, var(--color-fg-app) 50%, transparent);
  }

  .search-popup {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    z-index: 50;
    max-height: 320px;
    overflow-y: auto;
    background: var(--color-bg-card);
    border: 1px solid color-mix(in srgb, var(--color-fg-app) 12%, transparent);
    border-radius: var(--radius-input);
    box-shadow: var(--shadow-lg);
    padding: 4px;
  }

  .search-result {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    background: transparent;
    border: none;
    border-radius: var(--radius-chip);
    color: var(--color-fg-app);
    text-align: left;
    cursor: pointer;
    transition:
      background 80ms ease;
  }

  .search-result.active,
  .search-result:hover {
    background: color-mix(in srgb, var(--color-fg-app) 8%, transparent);
  }

  .search-result-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-fg-app) 8%, transparent);
    color: color-mix(in srgb, var(--color-fg-app) 80%, transparent);
  }

  .search-result-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .search-result-title {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--color-fg-app);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-result-meta {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
