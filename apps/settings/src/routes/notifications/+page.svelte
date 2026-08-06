<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import Rich from "@arlen/ui-kit/i18n/Rich.svelte";
  import { mark } from "@arlen/ui-kit/i18n";
  /// Notifications panel.
  ///
  /// Reads/writes:
  ///   * `~/.config/arlen/notifications.toml` (daemon rules)
  ///   * `~/.config/arlen/shell.toml [toast]` (visual rendering)
  ///
  /// Layout: vertical stack of grouped sections matching the Appearance
  /// panel pattern. Sections use the existing Group/Row primitives plus
  /// a few new components (TimeInput, DaysPicker, PositionPicker,
  /// AppPicker, AppRuleCard) that live alongside the Appearance helpers.

  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    Bell,
    BellOff,
    Coffee,
    Moon,
    Sunrise,
    Volume2,
    Trash2,
    AlertTriangle,
    Sparkles,
  } from "lucide-svelte";

  import {
    notifications,
    DND_MODE_LABELS,
    type DndMode,
    type ScheduleMode,
    type AppOverride,
  } from "$lib/stores/notifications";
  import { shell, type ToastPosition, type ToastAnimation } from "$lib/stores/shell";

  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { TimeInput } from "@arlen/ui-kit/components/ui/time-input";
  import { DaysPicker } from "@arlen/ui-kit/components/ui/days-picker";
  import { PositionPicker } from "@arlen/ui-kit/components/ui/position-picker";
  import AppPicker from "$lib/components/appearance/AppPicker.svelte";
  import AppRuleCard from "$lib/components/appearance/AppRuleCard.svelte";
  import { tauriAvailable } from "$lib/tauri";

  // What a scheduled quiet window still lets through.
  const SCHEDULE_MODE_OPTIONS = [
    { value: "priority", label: "Priority" },
    { value: "alarms", label: "Alarms" },
    { value: "total", label: "Total" },
  ];

  // ── Boot ───────────────────────────────────────────────────────────

  let knownApps = $state<string[]>([]);
  let unlisteners: UnlistenFn[] = [];

  async function refreshKnownApps() {
    try {
      const entries = await invoke<{ app_name: string }[]>(
        "notifications_get_known_apps",
      );
      knownApps = entries.map((e) => e.app_name);
    } catch (e) {
      console.error("[notifications] get_known_apps failed", e);
    }
  }

  onMount(() => {
    notifications.load();
    shell.load();
    refreshKnownApps();

    if (!tauriAvailable) return;
    listen("config:notifications:changed", () => notifications.load()).then(
      (fn) => unlisteners.push(fn),
    );
    listen("config:shell:changed", () => shell.load()).then((fn) =>
      unlisteners.push(fn),
    );

    return () => {
      for (const fn of unlisteners) fn();
    };
  });

  // ── Derived ─────────────────────────────────────────────────────────

  const dnd = $derived($notifications.data?.dnd ?? {});
  const schedule = $derived(dnd.schedule ?? {});
  const general = $derived($notifications.data?.general ?? {});
  const history = $derived($notifications.data?.history ?? {});
  const grouping = $derived($notifications.data?.grouping ?? {});
  const apps = $derived($notifications.data?.apps ?? {});
  const toast = $derived($shell.data?.toast ?? {});

  const dndMode = $derived<DndMode>(dnd.mode ?? "off");
  const expiresAt = $derived(dnd.expires_at);
  const expiresLabel = $derived.by(() => {
    if (!expiresAt) return null;
    const when = new Date(expiresAt);
    if (Number.isNaN(when.getTime())) return null;
    if (when.getTime() < Date.now()) return null;
    return when.toLocaleString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      weekday: "short",
    });
  });

  const alwaysAllow = $derived<string[]>(dnd.always_allow ?? []);
  const alwaysSuppress = $derived<string[]>(dnd.always_suppress ?? []);

  const appNames = $derived(Object.keys(apps).sort());
  const knownAppsForPicker = $derived.by(() => {
    const set = new Set(knownApps);
    for (const a of appNames) set.add(a);
    return [...set].sort();
  });

  // ── DND ─────────────────────────────────────────────────────────────

  const DND_PILLS: { mode: DndMode; icon: typeof BellOff }[] = [
    { mode: "off", icon: Bell },
    { mode: "priority", icon: AlertTriangle },
    { mode: "alarms", icon: Volume2 },
    { mode: "total", icon: BellOff },
    { mode: "scheduled", icon: Moon },
  ];

  async function setDndMode(mode: DndMode) {
    await notifications.setValue("dnd.mode", mode);
    if (mode === "off") {
      await notifications.setValue("dnd.expires_at", null);
    }
  }
  async function clearDndExpiry() {
    await notifications.setValue("dnd.expires_at", null);
  }
  async function setDndScheduleMode(mode: ScheduleMode) {
    await notifications.setValue("dnd.schedule.mode", mode);
  }
  async function setScheduleStart(value: string) {
    await notifications.setValue("dnd.schedule.start", value);
  }
  async function setScheduleEnd(value: string) {
    await notifications.setValue("dnd.schedule.end", value);
  }
  async function setScheduleDays(days: number[]) {
    await notifications.setValue("dnd.schedule.days", days);
  }
  async function setSuppressFullscreen(value: boolean) {
    await notifications.setValue("dnd.suppress_fullscreen", value);
  }

  async function dndForOneHour() {
    const expiry = await invoke<string>("notifications_dnd_expiry_in", {
      seconds: 3600,
    });
    await notifications.setValue("dnd.mode", "priority");
    await notifications.setValue("dnd.expires_at", expiry);
  }

  async function dndUntilMorning() {
    const expiry = await invoke<string>(
      "notifications_dnd_expiry_until_morning",
    );
    await notifications.setValue("dnd.mode", "priority");
    await notifications.setValue("dnd.expires_at", expiry);
  }

  // ── Lists ───────────────────────────────────────────────────────────

  async function addAlwaysAllow(name: string) {
    if (alwaysAllow.includes(name)) return;
    await notifications.setValue("dnd.always_allow", [...alwaysAllow, name]);
  }
  async function removeAlwaysAllow(name: string) {
    await notifications.setValue(
      "dnd.always_allow",
      alwaysAllow.filter((a) => a !== name),
    );
  }
  async function addAlwaysSuppress(name: string) {
    if (alwaysSuppress.includes(name)) return;
    await notifications.setValue("dnd.always_suppress", [
      ...alwaysSuppress,
      name,
    ]);
  }
  async function removeAlwaysSuppress(name: string) {
    await notifications.setValue(
      "dnd.always_suppress",
      alwaysSuppress.filter((a) => a !== name),
    );
  }

  // ── Toast Appearance ────────────────────────────────────────────────

  async function setToastPosition(value: ToastPosition) {
    await shell.setValue("toast.position", value);
  }
  async function setToastWidth(value: number) {
    await shell.setValue("toast.width", value);
  }
  async function setToastAnimation(value: ToastAnimation) {
    await shell.setValue("toast.animation", value);
  }

  // ── Timing / Grouping / History ─────────────────────────────────────

  async function setGeneral(key: string, value: number) {
    await notifications.setValue(`general.${key}`, value);
  }
  async function setGrouping(key: string, value: boolean | number) {
    await notifications.setValue(`grouping.${key}`, value);
  }
  async function setHistory(key: string, value: boolean | number) {
    await notifications.setValue(`history.${key}`, value);
  }

  let confirmingClear = $state(false);
  async function clearHistory() {
    if (!confirmingClear) {
      confirmingClear = true;
      setTimeout(() => (confirmingClear = false), 4000);
      return;
    }
    confirmingClear = false;
    try {
      await invoke("notifications_clear_history");
      await refreshKnownApps();
    } catch (e) {
      console.error("[notifications] clear_history failed", e);
    }
  }

  // ── Per-App ─────────────────────────────────────────────────────────

  let appFilter = $state("");
  const filteredApps = $derived.by(() => {
    const q = appFilter.trim().toLowerCase();
    if (!q) return appNames;
    return appNames.filter((a) => a.toLowerCase().includes(q));
  });

  async function addAppRule(name: string) {
    if (apps[name]) return;
    await notifications.setValue(`apps.${name}`, {});
  }
  async function patchAppRule(name: string, patch: Partial<AppOverride>) {
    const current = apps[name] ?? {};
    await notifications.setValue(`apps.${name}`, { ...current, ...patch });
  }
  async function removeAppRule(name: string) {
    await notifications.reset(`apps.${name}`);
  }

  // ── Test ────────────────────────────────────────────────────────────

  async function fireTest(priority: "low" | "normal" | "high" | "critical") {
    try {
      await invoke("notifications_test_notification", { priority });
    } catch (e) {
      console.error("[notifications] test failed", e);
    }
  }
</script>

<Page
  title={$t("s.notif.title")}
  description={$t("s.notif.desc")}
>
  <SectionGrid>
  <div class="span-full notif-column">

  {#if $notifications.loading && !$notifications.data}
    <div class="status">{$t("s.notif.loading")}</div>
  {:else if $notifications.error && !$notifications.data}
    <div class="error">
      {$t("s.notif.loadFailed", { error: $notifications.error })}
    </div>
  {:else}
    <div class="groups">
      <!-- ── DO NOT DISTURB ────────────────────────────────── -->
      <Section label={$t("s.notif.dnd")}>
        <div class="dnd-section">
          <div class="dnd-pills">
            {#each DND_PILLS as pill}
              {@const Icon = pill.icon}
              {@const meta = DND_MODE_LABELS[pill.mode]}
              {@const active = dndMode === pill.mode}
              <button
                type="button"
                class="dnd-pill"
                class:active
                aria-pressed={active}
                onclick={() => setDndMode(pill.mode)}
              >
                <span class="dnd-pill-icon"
                  ><Icon size={14} strokeWidth={2} /></span
                >
                <span class="dnd-pill-title">{$t(meta.title)}</span>
                <span class="dnd-pill-hint">{$t(meta.hint)}</span>
              </button>
            {/each}
          </div>

          {#if expiresLabel}
            <div class="expires-banner">
              <Sparkles size={12} strokeWidth={2.25} />
              <!-- The note here used to say this could not be one message
                   without dropping the <strong> around the timestamp. `Rich`
                   keeps it: the message marks a spot and the snippet renders the
                   same markup, so nothing about the look changes. -->
              <span>
                <Rich text={$t("s.notif.activeUntil", { time: mark("time") })}>
                  {#snippet time()}<strong>{expiresLabel}</strong>{/snippet}
                </Rich>
              </span>
              <button type="button" class="link" onclick={clearDndExpiry}
                >{$t("s.notif.clear")}</button
              >
            </div>
          {/if}

          <div class="quick-actions">
            <Button variant="outline" size="sm" onclick={dndForOneHour}>
              <Coffee size={12} strokeWidth={2} />
              {$t("s.notif.forOneHour")}
            </Button>
            <Button variant="outline" size="sm" onclick={dndUntilMorning}>
              <Sunrise size={12} strokeWidth={2} />
              {$t("s.notif.untilTomorrow")}
            </Button>
          </div>
        </div>

        {#if dndMode === "scheduled"}
          <Row label={$t("s.notif.scheduleMode")}>
            {#snippet control()}
              <SegmentedControl
                value={schedule.mode ?? "priority"}
                options={SCHEDULE_MODE_OPTIONS}
                ariaLabel={$t("s.notif.scheduleMode")}
                onchange={(v) => setDndScheduleMode(v as ScheduleMode)}
              />
            {/snippet}
          </Row>
          <Row label={$t("s.notif.from")}>
            {#snippet control()}
              <TimeInput
                value={schedule.start ?? "22:00"}
                onchange={setScheduleStart}
                ariaLabel={$t("s.notif.start")}
              />
            {/snippet}
          </Row>
          <Row label={$t("s.notif.until")}>
            {#snippet control()}
              <TimeInput
                value={schedule.end ?? "07:00"}
                onchange={setScheduleEnd}
                ariaLabel={$t("s.notif.end")}
              />
            {/snippet}
          </Row>
          <Row label={$t("s.notif.days")}>
            {#snippet control()}
              <DaysPicker
                value={schedule.days ?? []}
                onchange={setScheduleDays}
              />
            {/snippet}
          </Row>
        {/if}

        <Row label={$t("s.notif.suppressFs")} id="suppress-fullscreen">
          {#snippet control()}
            <Switch
              value={dnd.suppress_fullscreen ?? true}
              onchange={setSuppressFullscreen}
              ariaLabel={$t("s.notif.suppressFs")}
            />
          {/snippet}
        </Row>
      </Section>

      <!-- ── LISTS ────────────────────────────────── -->
      <Section label={$t("s.notif.lists")}>
        <Row label={$t("s.notif.alwaysAllow")} id="always-allow">
          {#snippet control()}
            <div class="list-control">
              <AppPicker
                knownApps={knownAppsForPicker}
                excluded={alwaysAllow}
                placeholder={$t("s.notif.addApp")}
                onpick={addAlwaysAllow}
              />
              {#if alwaysAllow.length > 0}
                <div class="chips">
                  {#each alwaysAllow as name}
                    <button
                      type="button"
                      class="chip"
                      onclick={() => removeAlwaysAllow(name)}>{name} ×</button
                    >
                  {/each}
                </div>
              {/if}
            </div>
          {/snippet}
        </Row>
        <Row label={$t("s.notif.alwaysSuppress")} id="always-suppress">
          {#snippet control()}
            <div class="list-control">
              <AppPicker
                knownApps={knownAppsForPicker}
                excluded={alwaysSuppress}
                placeholder={$t("s.notif.addApp")}
                onpick={addAlwaysSuppress}
              />
              {#if alwaysSuppress.length > 0}
                <div class="chips">
                  {#each alwaysSuppress as name}
                    <button
                      type="button"
                      class="chip muted"
                      onclick={() => removeAlwaysSuppress(name)}
                      >{name} ×</button
                    >
                  {/each}
                </div>
              {/if}
            </div>
          {/snippet}
        </Row>
      </Section>

      <!-- ── TOAST APPEARANCE ────────────────────────────────── -->
      <Section label={$t("s.notif.toastAppearance")}>
        <Row label={$t("s.notif.position")} id="toast-position">
          {#snippet control()}
            <PositionPicker
              value={toast.position ?? "top-right"}
              onchange={setToastPosition}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.width")} id="toast-width">
          {#snippet control()}
            <ValueSlider
              value={toast.width ?? 380}
              min={300}
              max={500}
              step={10}
              unit="px"
              ariaLabel={$t("s.notif.toastWidth")}
              onchange={setToastWidth}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.animation")} id="toast-animation">
          {#snippet control()}
            <div class="seg">
              {#each ["slide", "fade", "none"] as a (a)}
                <button
                  type="button"
                  class="seg-pill"
                  class:active={(toast.animation ?? "slide") === a}
                  onclick={() => setToastAnimation(a as ToastAnimation)}
                >
                  {a}
                </button>
              {/each}
            </div>
          {/snippet}
        </Row>
      </Section>

      <!-- ── TIMING ────────────────────────────────── -->
      <Section label={$t("s.notif.timing")}>
        <Row label={$t("s.notif.durNormal")} id="toast-duration-normal">
          {#snippet control()}
            <ValueSlider
              value={general.toast_duration_normal ?? 4000}
              min={1000}
              max={15000}
              step={500}
              unit="ms"
              ariaLabel={$t("s.notif.durNormal")}
              onchange={(v) => setGeneral("toast_duration_normal", v)}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.durHigh")} id="toast-duration-high">
          {#snippet control()}
            <ValueSlider
              value={general.toast_duration_high ?? 8000}
              min={3000}
              max={30000}
              step={1000}
              unit="ms"
              ariaLabel={$t("s.notif.durHigh")}
              onchange={(v) => setGeneral("toast_duration_high", v)}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.maxVisible")} id="max-visible">
          {#snippet control()}
            <ValueSlider
              value={general.max_visible_toasts ?? 5}
              min={1}
              max={10}
              step={1}
              unit=""
              ariaLabel={$t("s.notif.maxVisibleAria")}
              onchange={(v) => setGeneral("max_visible_toasts", v)}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.test")}>
          {#snippet control()}
            <div class="test-row">
              <Button variant="outline" size="sm" onclick={() => fireTest("normal")}>{$t("s.notif.normal")}</Button>
              <Button variant="outline" size="sm" onclick={() => fireTest("high")}>{$t("s.notif.high")}</Button>
              <Button variant="destructive" size="sm" onclick={() => fireTest("critical")}>{$t("s.notif.critical")}</Button>
            </div>
          {/snippet}
        </Row>
      </Section>

      <!-- ── GROUPING ────────────────────────────────── -->
      <Section label={$t("s.notif.grouping")}>
        <Row label={$t("s.notif.groupByApp")} id="group-by-app">
          {#snippet control()}
            <Switch
              value={grouping.by_app ?? true}
              onchange={(v) => setGrouping("by_app", v)}
              ariaLabel={$t("s.notif.groupByApp")}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.stackSimilar")} id="stack-similar">
          {#snippet control()}
            <Switch
              value={grouping.stack_similar ?? true}
              onchange={(v) => setGrouping("stack_similar", v)}
              ariaLabel={$t("s.notif.stackSimilar")}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.autoCollapse")} id="auto-collapse">
          {#snippet control()}
            <ValueSlider
              value={grouping.auto_collapse_after ?? 3}
              min={2}
              max={10}
              step={1}
              unit=""
              ariaLabel={$t("s.notif.autoCollapse")}
              onchange={(v) => setGrouping("auto_collapse_after", v)}
            />
          {/snippet}
        </Row>
      </Section>

      <!-- ── HISTORY ────────────────────────────────── -->
      <Section label={$t("s.notif.history")}>
        <Row label={$t("s.notif.keepHistory")} id="history-enabled">
          {#snippet control()}
            <Switch
              value={history.enabled ?? true}
              onchange={(v) => setHistory("enabled", v)}
              ariaLabel={$t("s.notif.keepHistory")}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.maxAge")} id="history-max-age">
          {#snippet control()}
            <ValueSlider
              value={history.max_age_days ?? 30}
              min={1}
              max={90}
              step={1}
              unit=" days"
              ariaLabel={$t("s.notif.maxAge")}
              onchange={(v) => setHistory("max_age_days", v)}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.maxCount")} id="history-max-count">
          {#snippet control()}
            <ValueSlider
              value={history.max_count ?? 1000}
              min={100}
              max={5000}
              step={100}
              unit=""
              ariaLabel={$t("s.notif.maxCount")}
              onchange={(v) => setHistory("max_count", v)}
            />
          {/snippet}
        </Row>
        <Row label={$t("s.notif.clearHistory")}>
          {#snippet control()}
            <Button variant="destructive" size="sm" onclick={clearHistory}>
              <Trash2 size={12} strokeWidth={2.25} />
              {confirmingClear ? $t("s.notif.confirmAgain") : $t("s.notif.clearHistoryBtn")}
            </Button>
          {/snippet}
        </Row>
      </Section>

      <!-- ── PER-APP ────────────────────────────────── -->
      <Section label={$t("s.notif.perApp")}>
        <div class="apps-section">
          <div class="apps-toolbar">
            <Input placeholder={$t("s.notif.filterRules")} bind:value={appFilter} />
            <AppPicker
              knownApps={knownAppsForPicker}
              excluded={appNames}
              placeholder={$t("s.notif.addRule")}
              onpick={addAppRule}
            />
          </div>

          {#if filteredApps.length === 0}
            <div class="apps-empty">
              {appFilter
                ? "No rules match this filter."
                : "No per-app rules yet. Pick an app above to override its priority, mute it, or block it entirely."}
            </div>
          {:else}
            <div class="apps-list">
              {#each filteredApps as name (name)}
                <AppRuleCard
                  appName={name}
                  rule={apps[name] ?? {}}
                  onchange={(patch) => patchAppRule(name, patch)}
                  onremove={() => removeAppRule(name)}
                />
              {/each}
            </div>
          {/if}
        </div>
      </Section>
    </div>
  {/if}
  </div>
  </SectionGrid>
</Page>

<style>
  .notif-column {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .groups {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .status {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .error {
    padding: 0.75rem 1rem;
    border-radius: var(--radius-input);
    border: 1px solid color-mix(in srgb, var(--color-error) 40%, transparent);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    color: var(--color-error);
    font-size: var(--text-sm);
  }

  /* ── DND ────────────────────────────────── */
  .dnd-section {
    padding: 0.75rem 0.75rem 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .dnd-pills {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 6px;
  }
  .dnd-pill {
    display: grid;
    grid-template-columns: 24px 1fr;
    grid-template-rows: auto auto;
    grid-column-gap: 8px;
    grid-row-gap: 1px;
    align-items: center;
    text-align: start;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    transition:
      background-color 120ms ease,
      border-color 120ms ease;
  }
  .dnd-pill:hover:not(.active) {
    background: color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .dnd-pill.active {
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 35%, transparent);
  }
  .dnd-pill-icon {
    grid-row: span 2;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .dnd-pill.active .dnd-pill-icon {
    color: var(--color-accent);
  }
  .dnd-pill-title {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--foreground);
  }
  .dnd-pill-hint {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    line-height: 1.25;
  }

  .expires-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--color-accent);
    font-size: var(--text-xs);
  }
  .expires-banner strong {
    color: var(--foreground);
    font-weight: 600;
  }
  .expires-banner .link {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    height: var(--height-control-compact, 24px);
    padding: 0 0.5rem;
    background: none;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-button);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font: inherit;
    font-size: var(--text-2xs);
    padding: 0;
  }

  .quick-actions {
    display: flex;
    gap: 6px;
  }

  /* ── Lists ────────────────────────────────── */
  .list-control {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 240px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    height: 22px;
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--foreground);
    font-size: var(--text-2xs);
    transition: background-color 120ms ease;
  }
  .chip:hover {
    background: color-mix(in srgb, var(--color-accent) 26%, transparent);
  }
  .chip.muted {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 18%, transparent);
  }
  .chip.muted:hover {
    background: color-mix(in srgb, var(--foreground) 18%, transparent);
  }

  .test-row {
    display: flex;
    gap: 6px;
  }

  /* ── Per-app section ────────────────────────────────── */
  .apps-section {
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
  }
  .apps-toolbar {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .apps-empty {
    padding: 0.75rem 0.6rem;
    text-align: center;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .apps-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
