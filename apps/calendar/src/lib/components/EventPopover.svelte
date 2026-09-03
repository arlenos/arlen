<script lang="ts">
  /// One event's details behind a click: the time in the reader's language,
  /// the place, the repetition sentence, the reminders it carries, and the
  /// honest chips - a UTC or foreign-zone time says so, a refused rule says
  /// "only this date". Editing goes through the dialog; the pencil is only
  /// offered where a surface can open it.
  import { Bell, MapPin, Pencil, Repeat } from "@lucide/svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { t, locale } from "$lib/i18n/messages";
  import { dayLabel, reminderLabel, repeatLabel } from "$lib/wording";
  import type { AgendaEvent } from "$lib/stores/calendar";
  import type { Snippet } from "svelte";

  let {
    event,
    onedit,
    children,
  }: {
    event: AgendaEvent;
    /// Opens the edit dialog; absent on read-only surfaces. A repeating event
    /// gets the three-way scope question first.
    onedit?: (e: AgendaEvent) => void;
    children: Snippet<[Record<string, unknown>]>;
  } = $props();

  // Absent is "this backend has not said", and then the line is not drawn:
  // "no reminders" would be a claim about a file nobody read that far.
  const reminders = $derived(event.alarms?.map((r) => reminderLabel(r, $t, $locale)) ?? []);

  // Owned here so the pencil can close the details before the dialog opens;
  // otherwise the popover sits on top of the form it just handed over to.
  let open = $state(false);
  function edit(): void {
    open = false;
    onedit?.(event);
  }
</script>

<Popover.Root bind:open>
  <Popover.Trigger>
    {#snippet child({ props })}
      {@render children(props)}
    {/snippet}
  </Popover.Trigger>
  <Popover.Content class="w-72">
    <div class="detail">
      <p class="d-title">{event.summary}</p>
      <p class="d-when">
        {dayLabel(event.date, $locale)}{#if event.time},
          {event.time}{#if event.end_time}&#8211;{event.end_time}{/if}{:else},
          {$t("cal.allDay")}{/if}
      </p>
      {#if event.tzid}
        <p class="d-line">{event.tzid}</p>
      {:else if event.kind === "utc"}
        <p class="d-line">{$t("cal.utc")}</p>
      {/if}
      {#if event.location}
        <p class="d-line"><MapPin size={12} strokeWidth={2} aria-hidden="true" /> {event.location}</p>
      {/if}
      {#if event.repeats}
        <p class="d-line"><Repeat size={12} strokeWidth={2} aria-hidden="true" /> {repeatLabel(event, $t)}</p>
        {#if !event.expanded}
          <p class="d-line quiet">{$t("cal.onlyThisOne")}</p>
        {/if}
      {/if}
      {#if reminders.length > 0}
        <p class="d-line"><Bell size={12} strokeWidth={2} aria-hidden="true" /> {reminders.join(", ")}</p>
      {/if}
      {#if onedit}
        <div class="d-actions">
          <Button variant="outline" size="sm" id="event-edit" onclick={edit}>
            <Pencil size={13} strokeWidth={1.75} />
            {$t("cal.edit.title")}
          </Button>
        </div>
      {/if}
    </div>
  </Popover.Content>
</Popover.Root>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .d-title {
    margin: 0;
    font-size: var(--text-sm, 13px);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .d-when {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .d-line {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .d-line :global(svg) {
    flex-shrink: 0;
  }
  .d-line.quiet {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .d-actions {
    display: flex;
    padding-top: 0.35rem;
  }
</style>
