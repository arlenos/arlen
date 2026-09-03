<script lang="ts">
  /// Creating an event, in a calendar's own form language: the title is the
  /// headline (borderless, focused, a placeholder - not a labelled field), and
  /// under it icon-led rows the way every calendar since forever writes them -
  /// when, how long, where, how often. No native date picker (a web idiom);
  /// the date button opens our own MiniMonth.
  ///
  /// The write is `calendar_create_event` - one VEVENT into the store
  /// directory, the watcher and the reminder daemon do the rest - and a
  /// refusal comes back as a sentence, not a pretence.
  import { Bell, CalendarDays, Clock, MapPin, Repeat } from "@lucide/svelte";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Toggle } from "@arlen/ui-kit/components/ui/toggle";
  import { TimeInput } from "@arlen/ui-kit/components/ui/time-input";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { DaysPicker } from "@arlen/ui-kit/components/ui/days-picker";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { t, locale } from "$lib/i18n/messages";
  import { dayLabel, reminderLabel } from "$lib/wording";
  import { parseQuick } from "$lib/quickparse";
  import {
    calendars,
    CALENDAR_PALETTE,
    createEvent,
    updateEvent,
    deleteEvent,
    remindersSupported,
    type AgendaEvent,
    type EventDraft,
    type Reminder,
  } from "$lib/stores/calendar";
  import { Trash2 } from "@lucide/svelte";
  import MiniMonth from "./MiniMonth.svelte";

  let {
    open,
    date,
    editing = null,
    editScope = "this",
    seed = null,
    onclose,
  }: {
    open: boolean;
    /// The focused date the form starts on.
    date: string;
    /// The event being edited, or null for a new one. A repeating event
    /// arrives here AFTER the three-way scope question; `editScope` carries
    /// the answer into the write.
    editing?: AgendaEvent | null;
    editScope?: "this" | "following" | "all";
    /// A slot handed over from the quick-create ("All options").
    seed?: { time: string; endTime: string; title: string } | null;
    onclose: () => void;
  } = $props();

  let summary = $state("");
  let day = $state("");
  let allDay = $state(false);
  let from = $state("09:00");
  let to = $state("10:00");
  let location = $state("");
  let repeat = $state<"none" | "daily" | "weekly">("none");
  /// DaysPicker speaks Monday-first indices 0..6.
  let onDays = $state<number[]>([]);
  let failed = $state<string | null>(null);
  let dateOpen = $state(false);
  let titleEl = $state<HTMLInputElement | null>(null);
  let calendarId = $state("");

  /// The reminders the event will carry. Presets are whole seconds before the
  /// start, toggled as chips; anything else arrives through the custom row
  /// and stays a chip of its own until it is toggled off again.
  let reminders = $state<Reminder[]>([]);
  const PRESETS = [0, 5 * 60, 10 * 60, 30 * 60, 3600, 86_400];
  let customOpen = $state(false);
  let customN = $state(15);
  let customUnit = $state<"minutes" | "hours" | "days">("minutes");
  let customRel = $state<"start" | "end">("start");
  const UNIT_SECONDS = { minutes: 60, hours: 3600, days: 86_400 };

  function isPreset(r: Reminder, s: number): boolean {
    return "seconds" in r.trigger && r.trigger.seconds === -s && r.trigger.related === "start";
  }
  function hasPreset(s: number): boolean {
    return reminders.some((r) => isPreset(r, s));
  }
  function setPreset(s: number, on: boolean): void {
    if (on && !hasPreset(s)) reminders = [...reminders, { trigger: { seconds: -s, related: "start" }, action: "DISPLAY" }];
    if (!on) reminders = reminders.filter((r) => !isPreset(r, s));
  }
  const custom = $derived(reminders.filter((r) => !PRESETS.some((s) => isPreset(r, s))));
  function presetLabel(s: number): string {
    return s === 0 ? $t("cal.remind.atStart") : reminderLabel({ trigger: { seconds: -s, related: "start" } }, $t, $locale);
  }
  function addCustom(): void {
    const seconds = -Math.max(1, Math.round(customN)) * UNIT_SECONDS[customUnit];
    const next: Reminder = { trigger: { seconds, related: customRel }, action: "DISPLAY" };
    const same = (r: Reminder) =>
      "seconds" in r.trigger && r.trigger.seconds === seconds && r.trigger.related === customRel;
    if (!reminders.some(same)) reminders = [...reminders, next];
    customOpen = false;
  }

  // The first calendar is the resting choice; follows the list arriving.
  $effect(() => {
    if (!calendarId && $calendars.length > 0) calendarId = $calendars[0].id;
  });
  const calColor = $derived($calendars.find((c) => c.id === calendarId)?.color ?? CALENDAR_PALETTE[0]);

  $effect(() => {
    if (open) {
      failed = null;
      if (editing) {
        summary = editing.summary;
        day = editing.date;
        allDay = editing.time === null;
        from = editing.time ?? "09:00";
        to = editing.end_time ?? (editing.time ? editing.time : "10:00");
        location = editing.location;
        repeat = "none";
        calendarId = editing.calendar ?? calendarId;
        reminders = editing.alarms ?? [];
      } else {
        // A new event starts blank every time: the fields must not carry an
        // edit that was cancelled a moment ago. The calendar choice stays,
        // the way the last-used calendar sticks in every calendar.
        day = date;
        summary = seed?.title ?? "";
        location = "";
        allDay = false;
        from = seed?.time ?? "09:00";
        to = seed?.endTime ?? "10:00";
        repeat = "none";
        onDays = [];
        reminders = [];
        customOpen = false;
      }
      setTimeout(() => titleEl?.focus(), 50);
    }
  });

  const DAY_NAMES = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

  /// The live parse of the title line (create mode only). Recognised pieces
  /// set the fields as they appear - the fields stay editable, and the words
  /// leave the title only when the event is written.
  const parsed = $derived(editing ? null : parseQuick(summary, new Date()));
  $effect(() => {
    if (!parsed) return;
    if (parsed.date) day = parsed.date;
    if (parsed.time) {
      allDay = false;
      from = parsed.time;
      to = parsed.endTime ?? parsed.time;
    }
    if (parsed.calendar) {
      const hit = $calendars.find(
        (c) => c.id.toLowerCase() === parsed.calendar || c.name.toLowerCase() === parsed.calendar,
      );
      if (hit) calendarId = hit.id;
    }
    if (parsed.location) location = parsed.location;
  });
  const anyParsed = $derived(
    parsed !== null && !!(parsed.date || parsed.time || parsed.location || parsed.calendar),
  );

  function reset(): void {
    summary = "";
    location = "";
    repeat = "none";
    onDays = [];
    reminders = [];
    customOpen = false;
  }

  async function submit(): Promise<void> {
    let refusal: string | null;
    if (editing) {
      refusal = await updateEvent(
        editing.uid,
        editing.calendar ?? calendarId,
        {
          summary: summary.trim(),
          date: day,
          allDay,
          time: allDay ? null : from,
          endTime: allDay ? null : to,
          location: location.trim(),
          // Only where the backend showed reminders: a list sent to one that
          // never said would be a write nobody can read back.
          alarms: $remindersSupported ? reminders : undefined,
        },
        editScope,
        editing.date,
      );
    } else {
      const draft: EventDraft = {
        summary: (parsed?.title || summary).trim(),
        date: day,
        allDay,
        time: allDay ? null : from,
        endTime: allDay ? null : to,
        location: location.trim(),
        repeat,
        onDays: repeat === "weekly" ? onDays.map((i) => DAY_NAMES[i]) : [],
        calendarId,
        alarms: reminders,
      };
      refusal = await createEvent(draft);
    }
    if (refusal) {
      failed = refusal;
      return;
    }
    reset();
    onclose();
  }

  async function remove(): Promise<void> {
    if (!editing) return;
    const refusal = await deleteEvent(editing.uid, editing.calendar ?? calendarId, editScope, editing.date);
    if (refusal) {
      failed = refusal;
      return;
    }
    reset();
    onclose();
  }
</script>

<Dialog {open} onClose={onclose} ariaLabel={editing ? $t("cal.edit.title") : $t("cal.form.title")}>
  <div class="form">
    <input
      bind:this={titleEl}
      bind:value={summary}
      id="event-summary"
      class="title"
      placeholder={$t("cal.form.titlePlaceholder")}
      aria-label={$t("cal.form.summary")}
    />

    {#if anyParsed && parsed}
      <p class="parsed" aria-live="polite">
        <span class="p-label">{$t("cal.parsed")}</span>
        {#if parsed.date}<span class="p-chip">{dayLabel(parsed.date, $locale)}</span>{/if}
        {#if parsed.time}<span class="p-chip">{parsed.time}{#if parsed.endTime}&#8211;{parsed.endTime}{/if}</span>{/if}
        {#if parsed.location}<span class="p-chip">{parsed.location}</span>{/if}
        {#if parsed.calendar}<span class="p-chip">/{parsed.calendar}</span>{/if}
      </p>
    {/if}

    <div class="row">
      <CalendarDays size={15} strokeWidth={1.75} aria-hidden="true" />
      <Popover.Root bind:open={dateOpen}>
        <Popover.Trigger>
          {#snippet child({ props })}
            <button type="button" class="date-btn" id="event-date" {...props}>
              {day ? dayLabel(day, $locale) : ""}
            </button>
          {/snippet}
        </Popover.Trigger>
        <Popover.Content class="w-64 p-1">
          <MiniMonth
            focus={day || date}
            marked={new Set()}
            onpick={(d) => {
              day = d;
              dateOpen = false;
            }}
          />
        </Popover.Content>
      </Popover.Root>
      <span class="row-end">
        <span class="quiet-label">{$t("cal.form.allDay")}</span>
        <Switch bind:value={allDay} ariaLabel={$t("cal.form.allDay")} />
      </span>
    </div>

    {#if !allDay}
      <div class="row">
        <Clock size={15} strokeWidth={1.75} aria-hidden="true" />
        <span class="times">
          <TimeInput value={from} ariaLabel={$t("cal.form.from")} onchange={(v) => (from = v)} />
          <span class="dash">&#8211;</span>
          <TimeInput value={to} ariaLabel={$t("cal.form.to")} onchange={(v) => (to = v)} />
        </span>
      </div>
    {/if}

    {#if $calendars.length > 1}
      <div class="row">
        <span class="cal-dot" style="background: {calColor}" aria-hidden="true"></span>
        <PopoverSelect
          value={calendarId}
          options={$calendars.map((c) => ({ value: c.id, label: c.name }))}
          width="180px"
          ariaLabel={$t("cal.form.calendar")}
          onchange={(v) => (calendarId = v)}
        />
      </div>
    {/if}

    <div class="row">
      <MapPin size={15} strokeWidth={1.75} aria-hidden="true" />
      <Input
        id="event-location"
        bind:value={location}
        placeholder={$t("cal.form.location")}
        aria-label={$t("cal.form.location")}
      />
    </div>

    <div class="row" class:top={repeat === "weekly"} class:gone={editing !== null}>
      <Repeat size={15} strokeWidth={1.75} aria-hidden="true" />
      <span class="repeat-col">
        <SegmentedControl
          id="event-repeat"
          bind:value={repeat}
          options={[
            { value: "none", label: $t("cal.form.repeat.none") },
            { value: "daily", label: $t("cal.form.repeat.daily") },
            { value: "weekly", label: $t("cal.form.repeat.weekly") },
          ]}
        />
        {#if repeat === "weekly"}
          <DaysPicker value={onDays} onchange={(v) => (onDays = v)} />
        {/if}
      </span>
    </div>

    <!-- Reminders, only where this backend carries them: the same gate that
         draws the bell in the popover opens this row, because a reminder
         written into a file nothing here can read back is not a reminder the
         person can check. -->
    {#if $remindersSupported}
      <div class="row top">
        <Bell size={15} strokeWidth={1.75} aria-hidden="true" />
        <span class="remind-col">
          <span class="chips" role="group" aria-label={$t("cal.form.reminders")}>
            {#each PRESETS as s (s)}
              <Toggle class="rchip" pressed={hasPreset(s)} onPressedChange={(p) => setPreset(s, p)}>
                {presetLabel(s)}
              </Toggle>
            {/each}
            {#each custom as r, i (i)}
              <Toggle class="rchip" pressed={true} onPressedChange={() => (reminders = reminders.filter((x) => x !== r))}>
                {reminderLabel(r, $t, $locale)}
              </Toggle>
            {/each}
            <Toggle class="rchip" bind:pressed={customOpen}>{$t("cal.remind.custom")}</Toggle>
          </span>
          {#if customOpen}
            <span class="custom">
              <NumberInput value={customN} min={1} max={999} step={1} ariaLabel={$t("cal.remind.custom")} onchange={(v) => (customN = v)} />
              <PopoverSelect
                value={customUnit}
                options={[
                  { value: "minutes", label: $t("cal.unit.minutes") },
                  { value: "hours", label: $t("cal.unit.hours") },
                  { value: "days", label: $t("cal.unit.days") },
                ]}
                width="120px"
                ariaLabel={$t("cal.unit.minutes")}
                onchange={(v) => (customUnit = v as typeof customUnit)}
              />
              <PopoverSelect
                value={customRel}
                options={[
                  { value: "start", label: $t("cal.remind.beforeTheStart") },
                  { value: "end", label: $t("cal.remind.beforeTheEnd") },
                ]}
                width="150px"
                ariaLabel={$t("cal.form.reminders")}
                onchange={(v) => (customRel = v as typeof customRel)}
              />
              <Button variant="outline" size="sm" id="event-reminder-add" onclick={addCustom}>{$t("cal.remind.add")}</Button>
            </span>
          {/if}
        </span>
      </div>
    {/if}

    <!-- Already a whole sentence in the reader's language: the store's
         refusal() writes it from the command's named problem. -->
    {#if failed}
      <p class="failed" role="alert">{failed}</p>
    {/if}

    <div class="actions">
      {#if editing}
        <Button variant="ghost" class="text-muted-foreground me-auto" id="event-delete" onclick={remove}>
          <Trash2 size={14} strokeWidth={1.75} />
          {$t("cal.delete")}
        </Button>
      {/if}
      <Button variant="ghost" id="event-cancel" onclick={onclose}>{$t("cal.form.cancel")}</Button>
      <Button id="event-create" onclick={submit}>{editing ? $t("cal.form.save") : $t("cal.form.create")}</Button>
    </div>
  </div>
</Dialog>

<style>
  /* The clock dialog's inset, the house register for a modal's inside. */
  .form {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
    padding: 1.1rem 1.25rem 1rem;
  }
  /* The headline, not a field: borderless with a quiet underline, like every
     calendar's quick entry. */
  .title {
    border: none;
    background: transparent;
    padding: 0.25rem 0.25rem 0.6rem;
    border-bottom: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: 0;
    font: inherit;
    font-size: var(--text-lg, 17px);
    font-weight: 600;
    color: var(--color-fg-primary, #fafafa);
  }
  .title::placeholder {
    color: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
    font-weight: 500;
  }
  .title:focus-visible {
    outline: none;
    border-bottom-color: var(--color-accent, #6366f1);
  }
  .parsed {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin: -0.35rem 0 0;
    font-size: var(--text-xs, 12px);
  }
  .p-label {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .p-chip {
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    min-height: var(--height-control, 30px);
  }
  .row.top {
    align-items: flex-start;
  }
  .row.top :global(svg) {
    margin-top: 0.45rem;
  }
  .row :global(svg) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .cal-dot {
    flex-shrink: 0;
    width: 0.7rem;
    height: 0.7rem;
    margin-inline: 2px;
    border-radius: var(--radius-chip, 4px);
  }
  .date-btn {
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font: inherit;
    font-size: var(--text-sm, 13px);
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .date-btn:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .date-btn:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .row-end {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-inline-start: auto;
  }
  .quiet-label {
    font-size: var(--text-sm, 13px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .times {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .dash {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .repeat-col,
  .remind-col {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    align-items: flex-start;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  /* A reminder chip: the kit toggle at chip size, its pressed face the
     selection tint, so on and off read at a glance across the row. */
  .chips :global(.rchip) {
    height: 1.6rem;
    padding: 0 0.6rem;
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: var(--radius-chip, 4px);
    font-size: var(--text-xs, 12px);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .chips :global(.rchip[data-state="on"]) {
    border-color: transparent;
    background: color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    color: var(--color-fg-primary);
  }
  .custom {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
  }
  .failed {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: var(--color-warning, #eab308);
  }
  .row.gone {
    display: none;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    padding-top: 0.35rem;
  }
</style>
