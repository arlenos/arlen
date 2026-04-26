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
  } from "$lib/components/ui/sidebar";
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
  } from "lucide-svelte";

  let query = $state("");
  let activeIndex = $state(0);
  const results = $derived<SearchResult[]>(
    query.trim().length > 0 ? search(query, 8) : [],
  );

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
  };

  // Group panels by section, matching the settings-app spec.
  const SECTIONS = [
    {
      label: "Personalization",
      panelIds: ["appearance", "display"] as const,
    },
    {
      label: "Input",
      panelIds: ["keyboard", "shortcuts", "mouse", "touchpad"] as const,
    },
    {
      label: "System",
      panelIds: ["notifications", "privacy", "extensions"] as const,
    },
    {
      label: "Info",
      panelIds: ["about"] as const,
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
        bind:value={query}
        onkeydown={onKey}
        placeholder="Search settings..."
        aria-label="Search settings"
        class="search-input group-data-[collapsible=icon]:w-8 group-data-[collapsible=icon]:px-0"
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
    border-radius: var(--radius-md);
    color: var(--color-fg-app);
    font-size: 0.875rem;
    outline: none;
    transition:
      background 120ms ease,
      border-color 120ms ease;
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
    border-radius: var(--radius-md);
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
    border-radius: var(--radius-sm);
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
    border-radius: var(--radius-sm);
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
