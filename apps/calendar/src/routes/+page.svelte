<script lang="ts">
  /// The calendar: week, month, day and the agenda around one store
  /// (calendar-app.md §6 - week default, year rejected). Every empty-looking
  /// state is a DIFFERENT state and says so; the honesty lines (service not
  /// arming reminders, unreadable files) live in the rail beside the views.
  ///
  /// Opened on a file, the app shows THAT file's agenda with the Keep action,
  /// exactly as before - the merge stays the person's.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen } from "@tauri-apps/api/event";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { ChevronLeft, ChevronRight } from "@lucide/svelte";
  import { tauriAvailable } from "$lib/tauri";
  import { initAppMenu, menuAction } from "$lib/menu";
  import { t, locale } from "$lib/i18n/messages";
  import { dayTitle, monthTitle, weekTitle } from "$lib/wording";
  import {
    agenda,
    loadAgenda,
    loadCalendars,
    hiddenCalendars,
    addDays,
    startOfWeek,
    ymd,
    updateEvent,
    type Agenda,
    type AgendaEvent,
  } from "$lib/stores/calendar";
  import CalendarSidebar from "$lib/components/CalendarSidebar.svelte";
  import QuickCreate from "$lib/components/QuickCreate.svelte";
  import RecurrenceScopeDialog, { type Scope } from "$lib/components/RecurrenceScopeDialog.svelte";
  import WeekView from "$lib/components/WeekView.svelte";
  import MonthView from "$lib/components/MonthView.svelte";
  import AgendaView from "$lib/components/AgendaView.svelte";
  import EventForm from "$lib/components/EventForm.svelte";

  type View = "week" | "three" | "month" | "day" | "agenda";
  let view = $state<View>("week");
  let focus = $state(ymd(new Date()));
  let creating = $state(false);
  /// The event the full dialog is editing, when it is.
  let editing = $state<AgendaEvent | null>(null);
  /// The quick-create panel, when a slot was spanned on the grid.
  let quick = $state<{ date: string; time: string; endTime: string; x: number; y: number } | null>(null);
  /// A slot the full dialog was seeded with from the quick-create.
  let seed = $state<{ date: string; time: string; endTime: string; title: string } | null>(null);

  /// The pending series question: which event, what for, and (for a drop)
  /// the changes waiting on the answer.
  let scopeAsk = $state<{
    event: AgendaEvent;
    action: "edit" | "delete" | "move";
    changes?: { date: string; time: string; endTime: string };
  } | null>(null);
  let editScope = $state<Scope>("this");

  function openEdit(e: AgendaEvent): void {
    if (e.repeats) {
      scopeAsk = { event: e, action: "edit" };
      return;
    }
    editScope = "this";
    editing = e;
    creating = true;
  }

  function moveRepeat(e: AgendaEvent, changes: { date: string; time: string; endTime: string }): void {
    scopeAsk = { event: e, action: "move", changes };
  }

  async function scopePicked(scope: Scope): Promise<void> {
    const ask = scopeAsk;
    scopeAsk = null;
    if (!ask) return;
    if (ask.action === "move" && ask.changes) {
      await updateEvent(ask.event.uid, ask.event.calendar ?? "", ask.changes, scope, ask.event.date);
      return;
    }
    editScope = scope;
    editing = ask.event;
    creating = true;
  }
  function quickMore(title: string): void {
    if (quick) seed = { date: quick.date, time: quick.time, endTime: quick.endTime, title };
    quick = null;
    editing = null;
    creating = true;
  }

  // The named cause, not a sentence: only the window is in the reader's
  // language. `other` stays for a failure the command cannot name.
  type Failure =
    | { problem: "launch"; reason: string }
    | { problem: "no-home" }
    | { problem: "unreadable"; why: string }
    | { problem: "other"; reason: string };
  let failure = $state<Failure | null>(null);

  /// The file the app was opened on, when it was opened on one. Read once: it
  /// is an argument, not a setting, and it cannot change while the window lives.
  let launched = $state<string | null>(null);

  /// The result of a keep, when one has been asked for. `null` before, so the
  /// button is the resting state rather than an empty sentence being one.
  type KeepProblem =
    | { problem: "not-a-file" }
    | { problem: "no-home" }
    | { problem: "cannot-make-dir"; why: string }
    | { problem: "already-kept"; name: string }
    | { problem: "copy-failed"; why: string };
  let kept = $state<{ path: string | null; problem: KeepProblem | null } | null>(null);

  /// Copy the opened file into the calendar directory, then read the directory
  /// rather than the file - the point of keeping it is that it is now one of
  /// yours, and the reminder daemon watches that folder.
  async function keep() {
    if (!launched) return;
    kept = await invoke<{ path: string | null; problem: KeepProblem | null }>("calendar_import", {
      path: launched,
    }).catch(() => ({ path: null, problem: null }));
    if (kept.path) {
      launched = null;
      await read();
    }
  }

  async function read() {
    try {
      await loadAgenda(launched);
      await loadCalendars();
      failure = null;
    } catch (e) {
      // The payload arrives as an object here; `apps/viewers` documents the
      // string-with-JSON shape, so both are accepted rather than one assumed.
      const named =
        e && typeof e === "object"
          ? (e as Record<string, unknown>)
          : (() => {
              const raw = String(e);
              const at = raw.indexOf("{");
              try {
                return at >= 0 ? (JSON.parse(raw.slice(at)) as Record<string, unknown>) : null;
              } catch {
                return null;
              }
            })();
      if (named?.problem === "no-home") failure = { problem: "no-home" };
      else if (named?.problem === "unreadable")
        failure = { problem: "unreadable", why: String(named.why ?? "") };
      else failure = { problem: "other", reason: String(e) };
    }
  }

  // The shell menu's dispatches, one at a time.
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "event.new") creating = true;
    else if (a === "go.today") focus = ymd(new Date());
    else if (a === "go.back") step(-1);
    else if (a === "go.forward") step(1);
    else if (a.startsWith("view.")) view = a.slice(5) as View;
  });

  onMount(() => {
    void initAppMenu();
    if (!tauriAvailable) {
      void read();
      return;
    }
    void (async () => {
      // A THROW AND A NULL ARE DIFFERENT ANSWERS: `null` is a window opened with
      // no file, a throw is the host failing to say what it was asked to open.
      // Swallowed together, somebody who double-clicked an invitation gets the
      // plain calendar and no sign that anything went wrong.
      try {
        launched = await invoke<string | null>("launch_file");
      } catch (e) {
        failure = { problem: "launch", reason: String(e) };
        return;
      }
      await read();
    })();
    // A file edited or synced while this window is open changes the answer, and
    // an agenda that keeps showing the old one gives no sign that it is stale.
    const stop = listen("arlen://calendar-changed", () => void read());
    return () => void stop.then((un) => un());
  });

  /// The events the visible calendars contribute; a hidden calendar's rows
  /// leave every view at once.
  const visibleEvents = $derived(
    ($agenda?.events ?? []).filter((e) => !$hiddenCalendars.has(e.calendar ?? "")),
  );
  const visibleAgenda = $derived($agenda ? { ...$agenda, events: visibleEvents } : null);

  const weekDays = $derived.by(() => {
    const monday = startOfWeek(focus);
    return Array.from({ length: 7 }, (_, i) => addDays(monday, i));
  });
  const threeDays = $derived(Array.from({ length: 3 }, (_, i) => addDays(focus, i)));

  const barTitle = $derived.by(() => {
    if (launched) return $t("cal.agenda");
    if (view === "month") return monthTitle(focus, $locale);
    if (view === "day") return dayTitle(focus, $locale);
    if (view === "week") return weekTitle(startOfWeek(focus), $locale);
    if (view === "three") return dayTitle(focus, $locale);
    return $t("cal.agenda");
  });

  function step(n: number): void {
    if (view === "week") focus = addDays(focus, 7 * n);
    else if (view === "day") focus = addDays(focus, n);
    else if (view === "three") focus = addDays(focus, 3 * n);
    else if (view === "month") {
      const [y, m] = focus.split("-").map(Number);
      const d = new Date(y, m - 1 + n, 1);
      focus = ymd(d);
    }
  }

  function openDay(d: string): void {
    focus = d;
    view = "day";
  }

  /// The desktop vocabulary: t today, arrows page, w/x/m/d/a switch views,
  /// c creates. Quiet while typing or while any dialog/popover is up.
  function globalKeys(e: KeyboardEvent): void {
    if (creating || quick || scopeAsk || launched) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest("input, textarea, [role='dialog']")) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const k = e.key;
    if (k === "t") focus = ymd(new Date());
    else if (k === "ArrowLeft" || k === "PageUp") step(-1);
    else if (k === "ArrowRight" || k === "PageDown") step(1);
    else if (k === "w") view = "week";
    else if (k === "x") view = "three";
    else if (k === "m") view = "month";
    else if (k === "d") view = "day";
    else if (k === "a") view = "agenda";
    else if (k === "c") creating = true;
    else return;
    e.preventDefault();
  }

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }

  async function startDrag(e: PointerEvent) {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* standalone (vite) has no toplevel to drag */
    }
  }

  async function toggleMax(e: MouseEvent) {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      /* no window in standalone */
    }
  }
</script>

<svelte:window onkeydown={globalKeys} />

<SidebarProvider class="h-screen min-h-0 overflow-hidden">
  <CalendarSidebar
    {focus}
    {launched}
    onpick={(d) => (focus = d)}
    oncreate={() => (creating = true)}
    onresult={(e) => {
      focus = e.date;
      view = "day";
    }}
  />

  <SidebarInset class="h-svh min-h-0">
    <!-- The header is a drag surface (a non-keyboard pointer interaction); its
         actual controls are the accessible buttons inside it, so the
         static-interaction lint is a false positive here. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header
      onpointerdown={startDrag}
      ondblclick={toggleMax}
      class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background px-2"
    >
      <SidebarTrigger class="-ml-1" />
      <Separator orientation="vertical" class="me-1 h-4" />
      <span class="select-none truncate text-sm font-medium text-foreground">{barTitle}</span>
      {#if !launched && view !== "agenda"}
        <IconAction label={$t("cal.prev")} size="control" onclick={() => step(-1)}>
          <ChevronLeft size={15} strokeWidth={1.75} />
        </IconAction>
        <IconAction label={$t("cal.next")} size="control" onclick={() => step(1)}>
          <ChevronRight size={15} strokeWidth={1.75} />
        </IconAction>
        <Button variant="ghost" size="sm" id="cal-today" onclick={() => (focus = ymd(new Date()))}>
          {$t("cal.todayButton")}
        </Button>
      {/if}
      <div class="flex-1"></div>
      {#if !launched}
        <SegmentedControl
          id="cal-view"
          size="compact"
          bind:value={view}
          options={[
            { value: "week", label: $t("cal.view.week") },
            { value: "three", label: $t("cal.view.threeDays") },
            { value: "month", label: $t("cal.view.month") },
            { value: "day", label: $t("cal.view.day") },
            { value: "agenda", label: $t("cal.view.agenda") },
          ]}
        />
      {/if}
      <WindowButtons />
    </header>

    <div class="content">
      {#if failure}
        <p class="note bad" role="alert">
          {#if failure.problem === "launch"}{$t("cal.failed.launch", { reason: failure.reason })}
          {:else if failure.problem === "no-home"}{$t("cal.failed.noHome")}
          {:else if failure.problem === "unreadable"}{$t("cal.failed.unreadable", { why: failure.why })}
          {:else}{$t("cal.failed.other", { reason: failure.reason })}{/if}
        </p>
      {:else if $agenda}
        {#if launched}
          <!-- The only way a calendar gets onto this machine today. Opening a
               file reads it where it lies, deliberately; an action rather than
               an automatic copy, so the merge stays the person's. -->
          <p class="keep">
            <button type="button" onclick={keep}>{$t("cal.keep")}</button>
          </p>
        {/if}
        {#if kept?.problem}
          <p class="note bad" role="alert">
            {#if kept.problem.problem === "not-a-file"}{$t("cal.keep.notAFile")}
            {:else if kept.problem.problem === "no-home"}{$t("cal.keep.noHome")}
            {:else if kept.problem.problem === "cannot-make-dir"}{$t("cal.keep.cannotMakeDir", { why: kept.problem.why })}
            {:else if kept.problem.problem === "already-kept"}{$t("cal.keep.alreadyKept", { name: kept.problem.name })}
            {:else}{$t("cal.keep.copyFailed", { why: kept.problem.why })}{/if}
          </p>
        {/if}

        {#if launched || view === "agenda"}
          <div class="scroll-list">
            <AgendaView agenda={visibleAgenda as Agenda} />
          </div>
        {:else if view === "week"}
          <WeekView
            days={weekDays}
            events={visibleEvents}
            onquick={(q) => (quick = q)}
            onedit={openEdit}
            onmoverepeat={moveRepeat}
          />
        {:else if view === "three"}
          <WeekView
            days={threeDays}
            events={visibleEvents}
            onquick={(q) => (quick = q)}
            onedit={openEdit}
            onmoverepeat={moveRepeat}
          />
        {:else if view === "day"}
          <WeekView
            days={[focus]}
            events={visibleEvents}
            onquick={(q) => (quick = q)}
            onedit={openEdit}
            onmoverepeat={moveRepeat}
          />
        {:else}
          <MonthView month={focus} events={visibleEvents} onopenday={openDay} onedit={openEdit} />
        {/if}
      {/if}
    </div>
  </SidebarInset>
</SidebarProvider>

<EventForm
  open={creating}
  date={seed?.date ?? focus}
  seed={seed ? { time: seed.time, endTime: seed.endTime, title: seed.title } : null}
  {editing}
  {editScope}
  onclose={() => {
    creating = false;
    editing = null;
    seed = null;
  }}
/>

{#if scopeAsk}
  <RecurrenceScopeDialog open={true} action={scopeAsk.action} onpick={scopePicked} oncancel={() => (scopeAsk = null)} />
{/if}

{#if quick}
  <QuickCreate
    at={{ x: quick.x, y: quick.y }}
    date={quick.date}
    time={quick.time}
    endTime={quick.endTime}
    onclose={() => (quick = null)}
    onmore={quickMore}
  />
{/if}

<style>
  .content {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .scroll-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .note {
    margin: 16px 14px;
    font-size: 13px;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .note.bad {
    color: var(--color-warning, #eab308);
  }
  .keep {
    margin: 16px 14px 0;
  }
  .keep button {
    padding: 6px 12px;
    font: inherit;
    font-size: 13px;
    color: var(--color-fg-primary, #e6e8ee);
    background: var(--color-bg-card, #171717);
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: 6px;
    cursor: pointer;
  }
  .keep button:hover {
    border-color: var(--color-border-strong, #3a3a3a);
  }
</style>
