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
  import { CalendarDays, Clock, MapPin, Repeat } from "@lucide/svelte";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { TimeInput } from "@arlen/ui-kit/components/ui/time-input";
  import { DaysPicker } from "@arlen/ui-kit/components/ui/days-picker";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { t, locale } from "$lib/i18n/messages";
  import { dayLabel } from "$lib/wording";
  import {
    calendars,
    CALENDAR_PALETTE,
    createEvent,
    updateEvent,
    deleteEvent,
    type AgendaEvent,
    type EventDraft,
  } from "$lib/stores/calendar";
  import { Trash2 } from "@lucide/svelte";
  import MiniMonth from "./MiniMonth.svelte";

  let {
    open,
    date,
    editing = null,
    seed = null,
    onclose,
  }: {
    open: boolean;
    /// The focused date the form starts on.
    date: string;
    /// The event being edited, or null for a new one. Repeating events do not
    /// reach this form until the series scope dialog exists (phase 3).
    editing?: AgendaEvent | null;
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
      } else {
        day = date;
        if (seed) {
          summary = seed.title;
          from = seed.time;
          to = seed.endTime;
          allDay = false;
        }
      }
      setTimeout(() => titleEl?.focus(), 50);
    }
  });

  const DAY_NAMES = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

  function reset(): void {
    summary = "";
    location = "";
    repeat = "none";
    onDays = [];
  }

  async function submit(): Promise<void> {
    let refusal: string | null;
    if (editing) {
      refusal = await updateEvent(editing.uid, editing.calendar ?? calendarId, {
        summary: summary.trim(),
        date: day,
        allDay,
        time: allDay ? null : from,
        endTime: allDay ? null : to,
        location: location.trim(),
      });
    } else {
      const draft: EventDraft = {
        summary: summary.trim(),
        date: day,
        allDay,
        time: allDay ? null : from,
        endTime: allDay ? null : to,
        location: location.trim(),
        repeat,
        onDays: repeat === "weekly" ? onDays.map((i) => DAY_NAMES[i]) : [],
        calendarId,
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
    const refusal = await deleteEvent(editing.uid, editing.calendar ?? calendarId);
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
  .repeat-col {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    align-items: flex-start;
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
