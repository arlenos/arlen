<script lang="ts">
  /// The agenda list, moved here whole: a first-class surface, not the
  /// accessibility alibi (calendar-app.md §7a c). Every sentence and state is
  /// the one the drive asserts on - the day headings through Intl, the today
  /// badge, the honest repeat sentences, "only this date" for a refused rule,
  /// the tz chips, and the two different kinds of empty, both naming the
  /// directory.
  import { MapPin, Repeat } from "@lucide/svelte";
  import { t, locale } from "$lib/i18n/messages";
  import { dayLabel, isToday, repeatLabel } from "$lib/wording";
  import type { Agenda, AgendaEvent } from "$lib/stores/calendar";

  let { agenda }: { agenda: Agenda } = $props();

  /// Events under their day, in the order the host sorted them.
  const days = $derived.by(() => {
    const out: { date: string; events: AgendaEvent[] }[] = [];
    for (const e of agenda.events) {
      const last = out[out.length - 1];
      if (last && last.date === e.date) last.events.push(e);
      else out.push({ date: e.date, events: [e] });
    }
    return out;
  });
</script>

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
      <h2>
        {dayLabel(day.date, $locale)}
        {#if isToday(day.date, new Date())}<span class="today">{$t("cal.today")}</span>{/if}
      </h2>
      <ul class="events">
        {#each day.events as e (e.uid + e.date + (e.time ?? ""))}
          <li class="event">
            <span class="when">
              {#if e.time}
                {e.time}{#if e.end_time}<span class="dash">&#8211;</span>{e.end_time}{/if}
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
                <span class="repeat">
                  <Repeat size={12} strokeWidth={2} />{repeatLabel(e, $t)}
                </span>
                {#if !e.expanded}
                  <span class="unexpanded">{$t("cal.onlyThisOne")}</span>
                {/if}
              {/if}
            </span>
          </li>
        {/each}
      </ul>
    </li>
  {/each}
</ul>

<style>
  .note {
    margin: 16px 14px;
    font-size: 13px;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .today {
    margin-inline-start: 8px;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-bg-app, #0f0f0f);
    background: var(--color-accent, #6366f1);
    vertical-align: middle;
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
  .unexpanded {
    font-size: 0.75rem;
    opacity: 0.75;
  }
  .repeat {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    color: var(--color-fg-secondary, #a3a3a3);
  }
</style>
