<script lang="ts">
  /// The calendars in the rail: one row per store file - colour dot, name, and
  /// the row itself toggles visibility (the universal calendar convention).
  /// The dot is its own button and opens the eight-colour palette; a hidden
  /// calendar's row steps back rather than disappearing.
  import { Check, Plus } from "@lucide/svelte";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { SidebarGroupLabel } from "@arlen/ui-kit/components/ui/sidebar";
  import { t } from "$lib/i18n/messages";
  import {
    calendars,
    calendarSets,
    hiddenCalendars,
    toggleCalendar,
    setCalendarColor,
    colorFailed,
    saveSet,
    applySet,
    CALENDAR_PALETTE,
  } from "$lib/stores/calendar";

  let naming = $state(false);
  let setName = $state("");

  function commitSet(): void {
    const name = setName.trim();
    if (name) saveSet(name);
    setName = "";
    naming = false;
  }
</script>

{#if $calendars.length > 0}
  <SidebarGroupLabel>{$t("cal.calendars")}</SidebarGroupLabel>
  <!-- Where the colour was chosen, so the sentence is beside the swatch that
       went back rather than somewhere else on the page. -->
  {#if $colorFailed}
    <Notice tone="error" class="mb-1.5" text={$colorFailed} />
  {/if}
  <div class="sets">
    <button type="button" class="set-chip" onclick={() => applySet(null)}>{$t("cal.sets.all")}</button>
    {#each $calendarSets as set (set.name)}
      <button type="button" class="set-chip" onclick={() => applySet(set)}>{set.name}</button>
    {/each}
    <IconAction label={$t("cal.sets.save")} onclick={() => (naming = true)}>
      <Plus size={13} strokeWidth={2} />
    </IconAction>
  </div>
  {#if naming}
    <div class="set-name">
      <Input
        bind:value={setName}
        placeholder={$t("cal.sets.name")}
        aria-label={$t("cal.sets.name")}
        onkeydown={(e: KeyboardEvent) => {
          if (e.key === "Enter") commitSet();
          else if (e.key === "Escape") (naming = false), (setName = "");
        }}
      />
    </div>
  {/if}
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
  .sets {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    padding: 0 0.5rem 0.35rem;
  }
  .set-chip {
    max-width: 8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0.15rem 0.55rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, currentColor 8%, transparent);
    font: inherit;
    font-size: var(--text-2xs, 11px);
    color: inherit;
  }
  .set-chip:hover {
    background: color-mix(in srgb, currentColor 14%, transparent);
  }
  .set-chip:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .set-name {
    padding: 0 0.5rem 0.4rem;
  }
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
  }
  .swatch.on {
    border-color: var(--color-fg-primary, #fff);
  }
</style>
