<script lang="ts">
  /// The calendar's rail: New event pinned on top, the month instrument, then
  /// the honesty lines that used to sit over the agenda (the service not
  /// arming reminders, unreadable files) - quiet, beside the content, not on
  /// top of it. Offcanvas rather than the icon rail: a mini month has no
  /// 3rem form.
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Plus } from "@lucide/svelte";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { t, locale } from "$lib/i18n/messages";
  import { dayLabel } from "$lib/wording";
  import { agenda, calendarMocked, calendars, colorOf, type AgendaEvent } from "$lib/stores/calendar";
  import MiniMonth from "./MiniMonth.svelte";
  import CalendarList from "./CalendarList.svelte";

  let {
    focus,
    launched,
    onpick,
    oncreate,
    onresult,
  }: {
    focus: string;
    /// The file the app was opened on; the service note is suppressed then.
    launched: string | null;
    onpick: (date: string) => void;
    oncreate: () => void;
    /// A search hit was chosen: jump the views to it.
    onresult: (e: AgendaEvent) => void;
  } = $props();

  let query = $state("");
  /// Hits over the loaded expansion window, chronological; the honesty line
  /// under the field names that reach until `calendar_search` (the whole-store
  /// seam) lands.
  const results = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return ($agenda?.events ?? [])
      .filter((e) => e.summary.toLowerCase().includes(q) || e.location.toLowerCase().includes(q))
      .slice(0, 50);
  });

  const marked = $derived(new Set(($agenda?.events ?? []).map((e) => e.date)));
</script>

<Sidebar>
  <!-- Search leads the rail from the h-10 band (level with the content bar);
       the app's own name used to sit here and said nothing the shell does not. -->
  <SidebarHeader class="h-10 justify-center py-0">
    <div class="head-search">
      <SearchField id="cal-search" bind:value={query} placeholder={$t("cal.search")} aria-label={$t("cal.search")} />
    </div>
  </SidebarHeader>
  <SidebarContent>
    <SidebarGroup>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton id="cal-new-event" onclick={oncreate}>
            <Plus strokeWidth={2} />
            <span>{$t("cal.newEvent")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarGroup>

    {#if query.trim() !== ""}
      <SidebarGroup class="pt-0">
        <p class="side-note">{$t("cal.search.range")}</p>
        <ul class="hits">
          {#each results as e (e.uid + e.date + (e.time ?? ""))}
            <li>
              <button type="button" class="hit" onclick={() => onresult(e)}>
                <span class="hit-dot" style="background: {colorOf($calendars, e)}" aria-hidden="true"></span>
                <span class="hit-body">
                  <span class="hit-title">{e.summary}</span>
                  <span class="hit-when">{dayLabel(e.date, $locale)}{#if e.time}, {e.time}{/if}</span>
                </span>
              </button>
            </li>
          {:else}
            <li class="side-note">{$t("cal.search.none")}</li>
          {/each}
        </ul>
      </SidebarGroup>
    {:else}
      <SidebarGroup class="pt-0">
        <MiniMonth {focus} {marked} {onpick} />
      </SidebarGroup>

      <SidebarGroup class="pt-0">
        <CalendarList />
      </SidebarGroup>
    {/if}

    <SidebarGroup class="pt-0">
      <!-- The house register for a fact beside the content: neutral for the
           sample, caution for a service that is not arming reminders, error
           for files that are missing from what is shown. -->
      <div class="side-notes">
        {#if $calendarMocked}
          <Notice tone="neutral" text={$t("cal.sample")} />
        {/if}
        {#if $agenda && !$agenda.service_running && !launched && !$calendarMocked}
          <Notice tone="caution" text={$t("cal.serviceDown")} />
        {/if}
        {#if $agenda && $agenda.unreadable > 0}
          <Notice tone="error" text={$t("cal.unreadable", { count: $agenda.unreadable })} />
        {/if}
      </div>
    </SidebarGroup>
  </SidebarContent>
  <SidebarRail />
</Sidebar>

<style>
  /* The field sits near the window's top-left corner, so its corners follow
     that corner concentrically: the window radius minus its inset in the
     h-10 band (the Settings-sidebar register). */
  .head-search {
    width: 100%;
    --search-radius: max(0px, calc(var(--radius-window, var(--radius-card)) - 0.375rem));
  }
  .hits {
    list-style: none;
    margin: 0;
    padding: 0 0.25rem;
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow-y: auto;
  }
  .hit {
    display: flex;
    width: 100%;
    align-items: flex-start;
    gap: 0.5rem;
    padding: 0.35rem 0.45rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font: inherit;
    text-align: start;
    color: inherit;
    cursor: pointer;
  }
  .hit:hover {
    background: color-mix(in srgb, currentColor 6%, transparent);
  }
  .hit:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }
  .hit-dot {
    flex-shrink: 0;
    width: 0.55rem;
    height: 0.55rem;
    margin-top: 0.3rem;
    border-radius: var(--radius-chip, 4px);
  }
  .hit-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .hit-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm, 13px);
  }
  .hit-when {
    font-size: var(--text-2xs, 11px);
    color: color-mix(in srgb, currentColor 55%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .side-notes {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0 4px;
  }
  .side-note {
    margin: 0 8px 4px;
    font-size: 11px;
    line-height: 1.4;
    color: color-mix(in srgb, currentColor 55%, transparent);
  }
  .side-note.bad {
    color: var(--color-warning, #eab308);
  }
</style>
