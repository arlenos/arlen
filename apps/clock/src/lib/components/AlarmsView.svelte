<script lang="ts">
  /// Alarms (clock-app.md §0.1): rows of daemon-owned alarms, an inline editor
  /// instead of a modal, and the honest wake state stated where alarms are SET
  /// (§2a) - a fact of the machine in plain tone, never a warning.
  import { Plus, Trash2 } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { TimeInput } from "@arlen/ui-kit/components/ui/time-input";
  import { DaysPicker } from "@arlen/ui-kit/components/ui/days-picker";
  import { clock, tick, setAlarm, toggleAlarm, deleteAlarm, type Alarm } from "$lib/stores/clock";
  import { fmtDays, fmtIn } from "$lib/format";
  import { t, locale } from "$lib/i18n/messages";

  // The open editor: an existing alarm's id, "new", or null.
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
</script>

<div class="al">
  {#if $clock && !$clock.wake_capable}
    <!-- The permanent product state, not an error: plain tone, at the place
         alarms are set. -->
    <p class="al-nowake">{$t("c.al.noWake")}</p>
  {/if}

  <div class="al-toolbar">
    <Button id="add-alarm" size="sm" onclick={() => openEditor()} disabled={editing !== null}>
      <Plus size={14} strokeWidth={2} />
      {$t("c.al.add")}
    </Button>
  </div>

  {#if editing === "new"}
    {@render editor()}
  {/if}

  {#if $clock}
    {#if $clock.alarms.length === 0 && editing === null}
      <p class="al-empty">{$t("c.al.empty")}</p>
    {/if}
    <div class="al-list">
      {#each $clock.alarms as a (a.id)}
        {#if editing === a.id}
          {@render editor()}
        {:else}
          <div class="al-row" class:off={!a.enabled}>
            <button type="button" class="al-main" onclick={() => openEditor(a)}>
              <span class="al-time">{a.time}</span>
              <span class="al-text">
                {#if a.label}<span class="al-label">{a.label}</span>{/if}
                <span class="al-days">{fmtDays(a.days, $locale, $t("c.al.everyDay"), $t("c.al.once"))}</span>
              </span>
              <span class="al-next">
                {#if a.enabled && a.next_fire_at}
                  {$t("c.al.ringsIn", { in: fmtIn(a.next_fire_at - $tick, $locale) })}
                {:else}
                  {$t("c.al.off")}
                {/if}
              </span>
            </button>
            <Switch
              value={a.enabled}
              ariaLabel={$t("c.al.toggleAria", { time: a.time })}
              onchange={(v) => toggleAlarm(a.id, v)}
            />
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

{#snippet editor()}
  <div class="al-editor">
    <div class="al-editor-grid">
      <span class="al-field-label">{$t("c.al.time")}</span>
      <TimeInput value={draft.time} onchange={(v) => (draft.time = v)} ariaLabel={$t("c.al.time")} />
      <span class="al-field-label">{$t("c.al.label")}</span>
      <Input
        id="alarm-label"
        class="al-label-input"
        value={draft.label}
        placeholder={$t("c.al.labelPlaceholder")}
        aria-label={$t("c.al.label")}
        oninput={(e: Event) => (draft.label = (e.currentTarget as HTMLInputElement).value)}
      />
      <span class="al-field-label">{$t("c.al.repeat")}</span>
      <DaysPicker value={draft.days} onchange={(v) => (draft.days = v)} />
      <span class="al-field-label"></span>
      <label class="al-late">
        <Switch value={draft.fire_late} size="sm" ariaLabel={$t("c.al.fireLate")} onchange={(v) => (draft.fire_late = v)} />
        <span>{$t("c.al.fireLate")}</span>
      </label>
    </div>
    {#if $clock && !$clock.wake_capable}
      <p class="al-nowake inset">{$t("c.al.noWake")}</p>
    {/if}
    <div class="al-editor-foot">
      {#if editing !== "new"}
        <Button variant="ghost" size="sm" class="text-muted-foreground" onclick={() => editing && deleteAlarm(editing).then(() => (editing = null))}>
          <Trash2 size={14} strokeWidth={1.75} />
          {$t("c.al.delete")}
        </Button>
      {/if}
      <span class="al-spacer"></span>
      <Button variant="ghost" size="sm" onclick={() => (editing = null)}>{$t("c.al.cancel")}</Button>
      <Button size="sm" onclick={save}>{$t("c.al.save")}</Button>
    </div>
  </div>
{/snippet}

<style>
  .al {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-width: 34rem;
    margin: 0 auto;
    padding: 0.9rem 1rem 1.5rem;
  }
  .al-nowake {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .al-nowake.inset {
    padding-top: 0.5rem;
  }
  .al-toolbar {
    display: flex;
  }
  .al-empty {
    margin: 0.75rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .al-list {
    display: flex;
    flex-direction: column;
  }
  .al-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .al-row:last-child {
    border-bottom: none;
  }
  .al-main {
    display: flex;
    align-items: baseline;
    gap: 0.9rem;
    flex: 1;
    min-width: 0;
    padding: 0.7rem 0.25rem;
    border: none;
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .al-time {
    font-size: var(--text-xl);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }
  .off .al-time {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .al-text {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    flex: 1;
    min-width: 0;
  }
  .al-label {
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
  }
  .al-days {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .al-next {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .al-editor {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.9rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .al-editor-grid {
    display: grid;
    grid-template-columns: 5.5rem minmax(0, 1fr);
    align-items: center;
    gap: 0.6rem 0.75rem;
  }
  .al-field-label {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .al-editor :global(.al-label-input) {
    max-width: 16rem;
  }
  .al-late {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .al-editor-foot {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .al-spacer {
    flex: 1;
  }
</style>
