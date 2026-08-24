<script lang="ts">
  /// The calendars in the rail: one row per store file - colour dot, name, and
  /// the row itself toggles visibility (the universal calendar convention).
  /// The dot is its own button and opens the eight-colour palette; a hidden
  /// calendar's row steps back rather than disappearing.
  import { Check } from "@lucide/svelte";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { SidebarGroupLabel } from "@arlen/ui-kit/components/ui/sidebar";
  import { t } from "$lib/i18n/messages";
  import {
    calendars,
    hiddenCalendars,
    toggleCalendar,
    setCalendarColor,
    CALENDAR_PALETTE,
  } from "$lib/stores/calendar";
</script>

{#if $calendars.length > 0}
  <SidebarGroupLabel>{$t("cal.calendars")}</SidebarGroupLabel>
  <ul class="cal-list">
    {#each $calendars as cal (cal.id)}
      {@const hidden = $hiddenCalendars.has(cal.id)}
      <li class="cal-row" class:hidden>
        <Popover.Root>
          <Popover.Trigger>
            {#snippet child({ props }: { props: Record<string, unknown> })}
              <button
                type="button"
                class="dot"
                style="background: {cal.color ?? CALENDAR_PALETTE[0]}"
                aria-label={$t("cal.calendarColor", { name: cal.name })}
                {...props}
              ></button>
            {/snippet}
          </Popover.Trigger>
          <Popover.Content class="w-auto p-2">
            <div class="palette">
              {#each CALENDAR_PALETTE as c (c)}
                <button
                  type="button"
                  class="swatch"
                  class:on={c === cal.color}
                  style="background: {c}"
                  aria-label={c}
                  onclick={() => setCalendarColor(cal.id, c)}
                ></button>
              {/each}
            </div>
          </Popover.Content>
        </Popover.Root>
        <button
          type="button"
          class="name"
          role="checkbox"
          aria-checked={!hidden}
          id={`cal-toggle-${cal.id}`}
          onclick={() => toggleCalendar(cal.id)}
        >
          <span class="label">{cal.name}</span>
          {#if !hidden}
            <Check size={13} strokeWidth={2} aria-hidden="true" />
          {/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .cal-list {
    list-style: none;
    margin: 0;
    padding: 0 0.25rem;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .cal-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    border-radius: var(--radius-input, 8px);
    padding-inline-start: 0.4rem;
  }
  .cal-row:hover {
    background: color-mix(in srgb, currentColor 6%, transparent);
  }
  .cal-row.hidden .label {
    opacity: 0.45;
  }
  .cal-row.hidden .dot {
    opacity: 0.35;
  }
  .dot {
    flex-shrink: 0;
    width: 0.7rem;
    height: 0.7rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    cursor: pointer;
  }
  .dot:focus-visible,
  .name:focus-visible,
  .swatch:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .name {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.4rem;
    border: none;
    background: transparent;
    font: inherit;
    font-size: var(--text-sm, 13px);
    text-align: start;
    color: inherit;
    cursor: pointer;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name :global(svg) {
    flex-shrink: 0;
    opacity: 0.6;
  }
  .palette {
    display: grid;
    grid-template-columns: repeat(4, 1.4rem);
    gap: 0.35rem;
  }
  .swatch {
    width: 1.4rem;
    height: 1.4rem;
    border: 2px solid transparent;
    border-radius: var(--radius-chip, 4px);
    cursor: pointer;
  }
  .swatch.on {
    border-color: var(--color-fg-primary, #fff);
  }
</style>
