<script lang="ts">
  /// The agenda: what is in your calendar files, grouped by the day each event
  /// writes for itself.
  ///
  /// Every empty-looking state here is a DIFFERENT state and says so. No host is
  /// not no files; no files is not no events; and a file that could not be read
  /// is counted out loud rather than quietly missing from the list. Three kinds
  /// of nothing rendered as one is the defect this app was written after.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { CalendarDays, MapPin, Repeat } from "@lucide/svelte";
  import { tauriAvailable } from "$lib/tauri";
  import { t, locale } from "$lib/i18n/messages";

  type AgendaEvent = {
    uid: string;
    summary: string;
    location: string;
    date: string;
    time: string | null;
    end_time: string | null;
    kind: string;
    tzid: string | null;
    repeats: boolean;
    every: string | null;
    every_n: number;
    on_days: string[];
    expanded: boolean;
  };
  type Agenda = {
    events: AgendaEvent[];
    directory: string;
    directory_exists: boolean;
    files: number;
    unreadable: number;
    /// False when the app read the files itself. The same agenda either way; the
    /// difference is that nothing is arming reminders.
    service_running: boolean;
  };

  let agenda = $state<Agenda | null>(null);
  let failure = $state<string | null>(null);

  /// The file the app was opened on, when it was opened on one. Read once: it
  /// is an argument, not a setting, and it cannot change while the window lives.
  let launched = $state<string | null>(null);

  /// The result of a keep, when one has been asked for. `null` before, so the
  /// button is the resting state rather than an empty sentence being one.
  let kept = $state<{ path: string | null; error: string | null } | null>(null);

  /// Copy the opened file into the calendar directory, then read the directory
  /// rather than the file - the point of keeping it is that it is now one of
  /// yours, and the reminder daemon watches that folder.
  async function keep() {
    if (!launched) return;
    kept = await invoke<{ path: string | null; error: string | null }>("calendar_import", {
      path: launched,
    }).catch((e) => ({ path: null, error: String(e) }));
    if (kept.path) {
      launched = null;
      await read();
    }
  }

  async function read() {
    try {
      agenda = await invoke<Agenda>("calendar_agenda", { file: launched });
      failure = null;
    } catch (e) {
      failure = String(e);
    }
  }

  onMount(() => {
    // Asked before the try, because "nobody to ask" is not a failure to catch.
    if (!tauriAvailable) return;
    void (async () => {
      launched = await invoke<string | null>("launch_file").catch(() => null);
      await read();
    })();
    // A file edited or synced while this window is open changes the answer, and
    // an agenda that keeps showing the old one gives no sign that it is stale.
    const stop = listen("arlen://calendar-changed", () => void read());
    return () => void stop.then((un) => un());
  });

  /// Events under their day, in the order the host sorted them.
  const days = $derived.by(() => {
    const out: { date: string; events: AgendaEvent[] }[] = [];
    for (const e of agenda?.events ?? []) {
      const last = out[out.length - 1];
      if (last && last.date === e.date) last.events.push(e);
      else out.push({ date: e.date, events: [e] });
    }
    return out;
  });

  /// The day heading, through Intl off the shared locale - never a catalogue
  /// string, so a German build says "Mittwoch, 19. August" rather than an
  /// English order with German words in it.
  function dayLabel(date: string): string {
    const [y, m, d] = date.split("-").map(Number);
    return new Intl.DateTimeFormat($locale, {
      weekday: "long",
      day: "numeric",
      month: "long",
    }).format(new Date(y, m - 1, d));
  }

  /// What the repeat chip says.
  ///
  /// "Repeats" alone was true of a standup every weekday and of a birthday every
  /// year, which is the same as saying nothing. The event carries the frequency,
  /// the interval and the weekdays as keys, and the sentence is written here so
  /// it is written in the reader's language.
  ///
  /// A rule the calendar refuses carries no frequency, and then the chip goes
  /// back to the bare word: better vague than wrong about somebody's week.
  const DAY_KEY: Record<string, string> = {
    mon: "cal.dayMon",
    tue: "cal.dayTue",
    wed: "cal.dayWed",
    thu: "cal.dayThu",
    fri: "cal.dayFri",
    sat: "cal.daySat",
    sun: "cal.daySun",
  };
  const EVERY_KEY: Record<string, string> = {
    daily: "cal.everyDaily",
    weekly: "cal.everyWeekly",
    monthly: "cal.everyMonthly",
    yearly: "cal.everyYearly",
  };

  function repeatLabel(e: { every: string | null; every_n: number; on_days: string[] }): string {
    const key = e.every ? EVERY_KEY[e.every] : undefined;
    if (!key) return $t("cal.repeats");
    const every = $t(key, { n: e.every_n });
    if (e.on_days.length === 0) return every;
    const days = e.on_days
      .map((d) => (DAY_KEY[d] ? $t(DAY_KEY[d]) : d))
      .join(", ");
    return $t("cal.onDays", { every, days });
  }
</script>

<main class="page">
  <header class="bar">
    <CalendarDays size={16} strokeWidth={2} />
    <h1>{$t("cal.agenda")}</h1>
    <span class="spacer"></span>
    <WindowButtons />
  </header>

  {#if !tauriAvailable}
    <p class="note">{$t("cal.hostAbsent")}</p>
  {:else if failure}
    <p class="note bad" role="alert">{$t("cal.failed", { reason: failure })}</p>
  {:else if agenda}
    <!-- Said whenever the service is absent, not only when the list is empty:
         the rows below are right either way, and what is missing is the arming
         of reminders, which nothing else on this screen would reveal. Opened on
         a single file the service is not involved at all, so there is nothing to
         report. -->
    {#if !agenda.service_running && !launched}
      <p class="note bad" role="status">{$t("cal.serviceDown")}</p>
    {/if}
    {#if launched}
      <!-- The only way a calendar gets onto this machine today. Opening a file
           reads it where it lies, deliberately; without this the directory stays
           empty on every install, the agenda is empty for everyone, and the
           reminder daemon watches a folder that never gains a file. An action
           rather than an automatic copy, so the merge stays the person's. -->
      <p class="keep">
        <button type="button" onclick={keep}>{$t("cal.keep")}</button>
      </p>
    {/if}
    {#if kept?.error}
      <p class="note bad" role="alert">{kept.error}</p>
    {/if}
    {#if agenda.unreadable > 0}
      <p class="note bad" role="alert">{$t("cal.unreadable", { count: agenda.unreadable })}</p>
    {/if}
    {#if agenda.events.length === 0 && agenda.unreadable === 0}
      <!-- Both name the directory: "put files somewhere" is not an instruction.
           No files and no events are different states - nothing has been put
           there, versus what is there holds nothing. -->
      <p class="note">
        {agenda.files === 0
          ? $t("cal.noFiles", { dir: agenda.directory })
          : $t("cal.empty", { dir: agenda.directory })}
      </p>
    {/if}
    <ul class="days">
      {#each days as day (day.date)}
        <li class="day">
          <h2>{dayLabel(day.date)}</h2>
          <ul class="events">
            {#each day.events as e (e.uid + e.date + (e.time ?? ""))}
              <li class="event">
                <span class="when">
                  {#if e.time}
                    {e.time}{#if e.end_time}<span class="dash">–</span>{e.end_time}{/if}
                  {:else}
                    {$t("cal.allDay")}
                  {/if}
                </span>
                <span class="what">
                  <span class="summary">{e.summary}</span>
                  <!-- A time written in UTC is not the reader's 16:00, and one
                       written floating is whatever clock they are reading. Both
                       say so; only a local zoned time can be shown bare. -->
                  {#if e.tzid}
                    <span class="tz">{e.tzid}</span>
                  {:else if e.kind === "utc"}
                    <span class="tz">{$t("cal.utc")}</span>
                  {/if}
                  {#if e.location}
                    <span class="where"><MapPin size={12} strokeWidth={2} />{e.location}</span>
                  {/if}
                  {#if e.repeats}
                    <!-- Said out loud: the rule is parsed but not worked out, so
                         showing this one occurrence silently would be a claim
                         about the others. -->
                    <span
                      class="repeat"
                      title={e.expanded ? $t("cal.repeatsShown") : $t("cal.repeatsUnexpanded")}
                    >
                      <Repeat size={12} strokeWidth={2} />{repeatLabel(e)}
                    </span>
                  {/if}
                </span>
              </li>
            {/each}
          </ul>
        </li>
      {/each}
    </ul>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
  }
  .page {
    min-height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
    font-family: "Inter Variable", Inter, system-ui, sans-serif;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--color-border-default, #262626);
  }
  .bar h1 {
    font-size: 13px;
    font-weight: 600;
    margin: 0;
  }
  .spacer {
    flex: 1;
  }
  .note {
    margin: 16px 14px;
    font-size: 13px;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .note.bad {
    color: var(--color-fg-warning, #eab308);
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
  .days,
  .events {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .day {
    padding: 10px 14px 4px;
  }
  .day h2 {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-fg-secondary, #a3a3a3);
    margin: 0 0 6px;
  }
  .event {
    display: flex;
    gap: 12px;
    padding: 7px 0;
    border-top: 1px solid var(--color-border-subtle, #1f1f1f);
    font-size: 13px;
  }
  .when {
    min-width: 96px;
    color: var(--color-fg-secondary, #a3a3a3);
    font-variant-numeric: tabular-nums;
  }
  .dash {
    padding: 0 2px;
  }
  .what {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .summary {
    font-weight: 500;
  }
  .tz,
  .where,
  .repeat {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    color: var(--color-fg-secondary, #a3a3a3);
  }
</style>
