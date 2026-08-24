<script lang="ts">
  /// Creating an event: title, date, all-day or a from/to, place, a simple
  /// repeat (never/daily/weekly with weekdays). The write is the intended
  /// `calendar_create_event` seam - one VEVENT into the store directory, the
  /// watcher and the reminder daemon do the rest - and until it exists a live
  /// press answers with the refusal instead of pretending.
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { TimeInput } from "@arlen/ui-kit/components/ui/time-input";
  import { DaysPicker } from "@arlen/ui-kit/components/ui/days-picker";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { t } from "$lib/i18n/messages";
  import { createEvent, type EventDraft } from "$lib/stores/calendar";

  let {
    open,
    date,
    onclose,
  }: {
    open: boolean;
    /// The focused date the form starts on.
    date: string;
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

  $effect(() => {
    if (open) {
      day = date;
      failed = null;
    }
  });

  const DAY_NAMES = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

  async function create(): Promise<void> {
    const draft: EventDraft = {
      summary: summary.trim(),
      date: day,
      allDay,
      time: allDay ? null : from,
      endTime: allDay ? null : to,
      location: location.trim(),
      repeat,
      onDays: repeat === "weekly" ? onDays.map((i) => DAY_NAMES[i]) : [],
    };
    const refusal = await createEvent(draft);
    if (refusal) {
      failed = refusal;
      return;
    }
    summary = "";
    location = "";
    repeat = "none";
    onDays = [];
    onclose();
  }
</script>

<Dialog {open} onClose={onclose} ariaLabel={$t("cal.form.title")}>
  <div class="form">
    <h2 class="f-title">{$t("cal.form.title")}</h2>
    <label class="field">
      <span class="k">{$t("cal.form.summary")}</span>
      <Input id="event-summary" bind:value={summary} />
    </label>
    <label class="field">
      <span class="k">{$t("cal.form.date")}</span>
      <Input id="event-date" type="date" bind:value={day} />
    </label>
    <label class="field">
      <span class="k">{$t("cal.form.allDay")}</span>
      <Switch bind:value={allDay} ariaLabel={$t("cal.form.allDay")} />
    </label>
    {#if !allDay}
      <div class="field">
        <span class="k">{$t("cal.form.from")}</span>
        <div class="times">
          <TimeInput value={from} ariaLabel={$t("cal.form.from")} onchange={(v) => (from = v)} />
          <span class="k">{$t("cal.form.to")}</span>
          <TimeInput value={to} ariaLabel={$t("cal.form.to")} onchange={(v) => (to = v)} />
        </div>
      </div>
    {/if}
    <label class="field">
      <span class="k">{$t("cal.form.location")}</span>
      <Input id="event-location" bind:value={location} />
    </label>
    <div class="field">
      <span class="k">{$t("cal.form.repeat")}</span>
      <SegmentedControl
        id="event-repeat"
        bind:value={repeat}
        options={[
          { value: "none", label: $t("cal.form.repeat.none") },
          { value: "daily", label: $t("cal.form.repeat.daily") },
          { value: "weekly", label: $t("cal.form.repeat.weekly") },
        ]}
      />
    </div>
    {#if repeat === "weekly"}
      <div class="field">
        <span class="k"></span>
        <DaysPicker value={onDays} onchange={(v) => (onDays = v)} />
      </div>
    {/if}
    {#if failed}
      <p class="failed" role="alert">{$t("cal.form.failed", { reason: failed })}</p>
    {/if}
    <div class="actions">
      <Button id="event-create" onclick={create}>{$t("cal.form.create")}</Button>
      <Button variant="ghost" id="event-cancel" onclick={onclose}>{$t("cal.form.cancel")}</Button>
    </div>
  </div>
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .f-title {
    margin: 0 0 0.25rem;
    font-size: var(--text-base, 15px);
    font-weight: 600;
  }
  .field {
    display: grid;
    grid-template-columns: 6rem 1fr;
    align-items: center;
    gap: 0.6rem;
  }
  .k {
    font-size: var(--text-sm, 13px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .times {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .failed {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: var(--color-warning, #eab308);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    padding-top: 0.25rem;
  }
</style>
