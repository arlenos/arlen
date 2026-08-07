<script lang="ts">
  /// Alarms: identical tiles (rule 5) - big time on ONE left edge (rule 3),
  /// one comma-joined metadata line a step down (rule 6), the switch in one
  /// trailing column. Add/edit happens in a DIALOG (rule 7), opened from the
  /// chrome "+" or by clicking a tile. The no-wake fact stays where alarms are
  /// set (clock-app.md §2a): over the list and inside the dialog, plain tone.
  import { Trash2 } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { TimeInput } from "@arlen/ui-kit/components/ui/time-input";
  import { DaysPicker } from "@arlen/ui-kit/components/ui/days-picker";
  import { clock, tick, setAlarm, toggleAlarm, deleteAlarm, type Alarm } from "$lib/stores/clock";
  import { addSignal } from "$lib/stores/ui";
  import { fmtDays, fmtIn } from "$lib/format";
  import { t, locale } from "$lib/i18n/messages";

  // The dialog: an existing alarm's id, "new", or null (closed).
  let editing = $state<string | null>(null);
  let draft = $state({ time: "07:00", label: "", days: [] as number[], fire_late: false });

  function openEditor(a?: Alarm): void {
    if (a) {
      editing = a.id;
      draft = { time: a.time, label: a.label, days: [...a.days], fire_late: a.fire_late };
    } else {
      editing = "new";
      draft = { time: "07:00", label: "", days: [], fire_late: false };
    }
  }
  async function save(): Promise<void> {
    const id = editing === "new" ? `a-${Date.now()}` : (editing ?? "");
    await setAlarm({ id, time: draft.time, label: draft.label, days: draft.days, enabled: true, fire_late: draft.fire_late });
    editing = null;
  }

  // The chrome "+" pings; ignore the value already present on mount.
  let seenAdd = $state(-1);
  $effect(() => {
    const n = $addSignal;
    if (seenAdd === -1) {
      seenAdd = n;
      return;
    }
    if (n !== seenAdd) {
      seenAdd = n;
      openEditor();
    }
  });

  const metaLine = (a: Alarm) =>
    [a.label, fmtDays(a.days, $locale, $t("c.al.everyDay"), $t("c.al.once"))].filter(Boolean).join(", ");
</script>

<div class="al">
  {#if $clock && !$clock.wake_capable}
    <!-- The permanent product state, not an error: plain tone, at the place
         alarms are set. -->
    <p class="al-nowake">{$t("c.al.noWake")}</p>
  {/if}

  {#if $clock}
    {#if $clock.alarms.length === 0}
      <p class="al-empty">{$t("c.al.empty")}</p>
    {/if}
    <div class="al-list">
      {#each $clock.alarms as a (a.id)}
        <div class="tile" class:off={!a.enabled}>
          <button type="button" class="al-main" onclick={() => openEditor(a)}>
            <span class="al-time">{a.time}</span>
            <span class="al-text">
              <span class="al-meta">{metaLine(a)}</span>
              <span class="al-next">
                {#if a.enabled && a.next_fire_at}
                  {$t("c.al.ringsIn", { in: fmtIn(a.next_fire_at - $tick, $locale) })}
                {:else}
                  {$t("c.al.off")}
                {/if}
              </span>
            </span>
          </button>
          <Switch
            value={a.enabled}
            ariaLabel={$t("c.al.toggleAria", { time: a.time })}
            onchange={(v) => toggleAlarm(a.id, v)}
          />
        </div>
      {/each}
    </div>
  {/if}
</div>

<Dialog open={editing !== null} onClose={() => (editing = null)} ariaLabel={$t("c.al.add")} size="md">
  <div class="al-dialog">
    <div class="al-grid">
      <span class="al-field-label">{$t("c.al.time")}</span>
      <TimeInput value={draft.time} onchange={(v) => (draft.time = v)} ariaLabel={$t("c.al.time")} />
      <span class="al-field-label">{$t("c.al.label")}</span>
      <Input
        id="alarm-label"
        value={draft.label}
        placeholder={$t("c.al.labelPlaceholder")}
        aria-label={$t("c.al.label")}
        oninput={(e: Event) => (draft.label = (e.currentTarget as HTMLInputElement).value)}
      />
      <span class="al-field-label">{$t("c.al.repeat")}</span>
      <DaysPicker value={draft.days} onchange={(v) => (draft.days = v)} />
    </div>
    <label class="al-late">
      <Switch value={draft.fire_late} size="sm" ariaLabel={$t("c.al.fireLate")} onchange={(v) => (draft.fire_late = v)} />
      <span>{$t("c.al.fireLate")}</span>
    </label>
    {#if $clock && !$clock.wake_capable}
      <p class="al-nowake dialog">{$t("c.al.noWake")}</p>
    {/if}
    <div class="al-foot">
      {#if editing !== "new" && editing !== null}
        <Button
          variant="ghost"
          size="sm"
          class="text-muted-foreground"
          onclick={() => editing && deleteAlarm(editing).then(() => (editing = null))}
        >
          <Trash2 size={14} strokeWidth={1.75} />
          {$t("c.al.delete")}
        </Button>
      {/if}
      <span class="al-spacer"></span>
      <Button variant="ghost" size="sm" onclick={() => (editing = null)}>{$t("c.al.cancel")}</Button>
      <Button size="sm" onclick={save}>{$t("c.al.save")}</Button>
    </div>
  </div>
</Dialog>

<style>
  .al {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-width: 30rem;
    margin: 0 auto;
    padding: 1.1rem 1rem 1.5rem;
  }
  .al-nowake {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .al-nowake.dialog {
    text-align: start;
  }
  .al-empty {
    margin: 0.75rem 0 0;
    font-size: var(--text-sm);
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .al-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  /* One alarm, one tile - the shared tile geometry. */
  .tile {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding-inline-end: 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .al-main {
    display: flex;
    align-items: center;
    gap: 1rem;
    flex: 1;
    min-width: 0;
    padding: 0.85rem 1rem;
    border: none;
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .al-time {
    /* All tile times share one size and one left edge. */
    flex-shrink: 0;
    min-width: 5.2rem;
    font-size: var(--clock-list-time, 1.75rem);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }
  .off .al-time {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .al-text {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    flex: 1;
    min-width: 0;
  }
  .al-meta {
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .off .al-meta {
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .al-next {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }

  .al-dialog {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1.1rem 1.25rem 1rem;
  }
  .al-grid {
    display: grid;
    grid-template-columns: 4.5rem minmax(0, 1fr);
    align-items: center;
    gap: 0.6rem 0.75rem;
  }
  .al-field-label {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .al-late {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .al-foot {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding-top: 0.25rem;
  }
  .al-spacer {
    flex: 1;
  }
</style>
