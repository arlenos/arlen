<script lang="ts">
  /// The quick-create at the spot a slot was spanned: a title, the spanned
  /// time said back, Enter creates into the chosen calendar, and "All
  /// options" hands the draft to the full dialog. Deliberately tiny - the
  /// dialog exists for everything else.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t, locale } from "$lib/i18n/messages";
  import { dayLabel } from "$lib/wording";
  import { calendars, colorOf, createEvent } from "$lib/stores/calendar";

  let {
    at,
    date,
    time,
    endTime,
    onclose,
    onmore,
  }: {
    at: { x: number; y: number };
    date: string;
    time: string;
    endTime: string;
    onclose: () => void;
    /// Open the full dialog seeded with this slot.
    onmore: (title: string) => void;
  } = $props();

  let title = $state("");
  let failed = $state<string | null>(null);
  let el = $state<HTMLElement | null>(null);
  let input = $state<HTMLInputElement | null>(null);

  $effect(() => {
    setTimeout(() => input?.focus(), 30);
  });

  /// Clamped so the panel never leaves the window.
  const pos = $derived.by(() => {
    const w = 300;
    const h = 150;
    const x = Math.min(Math.max(8, at.x - w / 2), window.innerWidth - w - 8);
    const y = Math.min(Math.max(8, at.y + 10), window.innerHeight - h - 8);
    return { x, y };
  });

  const calColor = $derived($calendars.length > 0 ? colorOf($calendars, { calendar: $calendars[0].id } as never) : null);

  async function create(): Promise<void> {
    const refusal = await createEvent({
      summary: title.trim(),
      date,
      allDay: false,
      time,
      endTime,
      location: "",
      repeat: "none",
      onDays: [],
      calendarId: $calendars[0]?.id ?? "",
      // No reminder, which is what the full editor gives a NEW event too
      // (EventForm sets `reminders = []` on open). A quick-created event
      // that arrived with a reminder the same event made the long way does
      // not have would be two answers to one question.
      alarms: [],
    });
    if (refusal) {
      failed = refusal;
      return;
    }
    onclose();
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      e.preventDefault();
      void create();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }
</script>

<!-- A light dismiss layer under the panel; the panel itself handles keys. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="qc-layer" onpointerdown={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="qc"
    bind:this={el}
    style="left: {pos.x}px; top: {pos.y}px;"
    role="dialog"
    aria-label={$t("cal.form.title")}
  >
    <input
      bind:this={input}
      bind:value={title}
      class="qc-title"
      placeholder={$t("cal.form.titlePlaceholder")}
      aria-label={$t("cal.form.summary")}
      onkeydown={onKeydown}
    />
    <p class="qc-when">
      {#if calColor}<span class="qc-dot" style="background: {calColor}" aria-hidden="true"></span>{/if}
      {dayLabel(date, $locale)}, {time}&#8211;{endTime}
    </p>
    {#if failed}
      <p class="qc-failed" role="alert">{failed}</p>
    {/if}
    <div class="qc-actions">
      <Button variant="ghost" size="sm" id="quick-more" onclick={() => onmore(title)}>
        {$t("cal.quick.moreOptions")}
      </Button>
      <Button size="sm" id="quick-create" onclick={create}>{$t("cal.form.create")}</Button>
    </div>
  </div>
</div>

<style>
  .qc-layer {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .qc {
    position: fixed;
    width: 300px;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.8rem 0.9rem;
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: var(--radius-modal, 16px);
    background: var(--color-bg-card, #171717);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.35);
  }
  .qc-title {
    border: none;
    background: transparent;
    padding: 0.15rem 0.1rem 0.4rem;
    border-bottom: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: 0;
    font: inherit;
    font-size: var(--text-base, 15px);
    font-weight: 600;
    color: var(--color-fg-primary, #fafafa);
  }
  .qc-title:focus-visible {
    outline: none;
    border-bottom-color: var(--color-accent, #6366f1);
  }
  .qc-title::placeholder {
    color: var(--color-fg-secondary, #a3a3a3);
    font-weight: 500;
  }
  .qc-when {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .qc-dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: var(--radius-chip, 4px);
  }
  .qc-failed {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: var(--color-warning, #eab308);
  }
  .qc-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
  }
</style>
