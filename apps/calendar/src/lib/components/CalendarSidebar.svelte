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
  import { t } from "$lib/i18n/messages";
  import { agenda, calendarMocked } from "$lib/stores/calendar";
  import MiniMonth from "./MiniMonth.svelte";
  import CalendarList from "./CalendarList.svelte";

  let {
    focus,
    launched,
    onpick,
    oncreate,
  }: {
    focus: string;
    /// The file the app was opened on; the service note is suppressed then.
    launched: string | null;
    onpick: (date: string) => void;
    oncreate: () => void;
  } = $props();

  const marked = $derived(new Set(($agenda?.events ?? []).map((e) => e.date)));
</script>

<Sidebar>
  <SidebarHeader class="h-10 flex-row items-center py-0">
    <span class="px-2 text-[0.6875rem] font-semibold uppercase tracking-[0.1em] text-sidebar-foreground/55">
      {$t("cal.app.title")}
    </span>
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

    <SidebarGroup class="pt-0">
      <MiniMonth {focus} {marked} {onpick} />
    </SidebarGroup>

    <SidebarGroup class="pt-0">
      <CalendarList />
    </SidebarGroup>

    <SidebarGroup class="pt-0">
      {#if $calendarMocked}
        <p class="side-note">{$t("cal.sample")}</p>
      {/if}
      {#if $agenda && !$agenda.service_running && !launched && !$calendarMocked}
        <p class="side-note bad" role="status">{$t("cal.serviceDown")}</p>
      {/if}
      {#if $agenda && $agenda.unreadable > 0}
        <p class="side-note bad" role="alert">{$t("cal.unreadable", { count: $agenda.unreadable })}</p>
      {/if}
    </SidebarGroup>
  </SidebarContent>
  <SidebarRail />
</Sidebar>

<style>
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
