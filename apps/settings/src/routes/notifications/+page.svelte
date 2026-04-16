<script lang="ts">
  /// Notifications panel.
  ///
  /// Reads/writes:
  ///   * `~/.config/lunaris/notifications.toml` (daemon rules)
  ///   * `~/.config/lunaris/shell.toml [toast]` (visual rendering)
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

  import Group from "$lib/components/appearance/Group.svelte";
  import Row from "$lib/components/appearance/Row.svelte";
  import { Switch } from "$lib/components/ui/switch";
  import ValueSlider from "$lib/components/appearance/ValueSlider.svelte";
  import TimeInput from "$lib/components/appearance/TimeInput.svelte";
  import DaysPicker from "$lib/components/appearance/DaysPicker.svelte";
  import PositionPicker from "$lib/components/appearance/PositionPicker.svelte";
  import AppPicker from "$lib/components/appearance/AppPicker.svelte";
  import AppRuleCard from "$lib/components/appearance/AppRuleCard.svelte";

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

<div class="page">
  <header class="head">
    <h1>Notifications</h1>
    <p class="lede">
      Control how Lunaris delivers notifications, when to stay quiet, and
      which apps get special treatment.
    </p>
  </header>

  {#if $notifications.loading && !$notifications.data}
    <div class="status">Loading…</div>
  {:else if $notifications.error && !$notifications.data}
    <div class="error">
      Failed to load notifications config: {$notifications.error}
    </div>
  {:else}
    <div class="groups">
      <!-- ── DO NOT DISTURB ────────────────────────────────── -->
      <Group label="Do Not Disturb">
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
                <span class="dnd-pill-title">{meta.title}</span>
                <span class="dnd-pill-hint">{meta.hint}</span>
              </button>
            {/each}
          </div>

          {#if expiresLabel}
            <div class="expires-banner">
              <Sparkles size={12} strokeWidth={2.25} />
              <span>Active until <strong>{expiresLabel}</strong></span>
              <button type="button" class="link" onclick={clearDndExpiry}
                >Clear</button
              >
            </div>
          {/if}

          <div class="quick-actions">
            <button type="button" class="quick-btn" onclick={dndForOneHour}>
              <Coffee size={12} strokeWidth={2} />
              <span>For 1 hour</span>
            </button>
            <button type="button" class="quick-btn" onclick={dndUntilMorning}>
              <Sunrise size={12} strokeWidth={2} />
              <span>Until tomorrow</span>
            </button>
          </div>
        </div>

        {#if dndMode === "scheduled"}
          <Row label="Schedule mode">
            {#snippet control()}
              <div class="seg">
                {#each ["priority", "alarms", "total"] as m (m)}
                  <button
                    type="button"
                    class="seg-pill"
                    class:active={(schedule.mode ?? "priority") === m}
                    onclick={() => setDndScheduleMode(m as ScheduleMode)}
                  >
                    {m}
                  </button>
                {/each}
              </div>
            {/snippet}
          </Row>
          <Row label="From">
            {#snippet control()}
              <TimeInput
                value={schedule.start ?? "22:00"}
                onchange={setScheduleStart}
                ariaLabel="Start"
              />
            {/snippet}
          </Row>
          <Row label="Until">
            {#snippet control()}
              <TimeInput
                value={schedule.end ?? "07:00"}
                onchange={setScheduleEnd}
                ariaLabel="End"
              />
            {/snippet}
          </Row>
          <Row label="Days">
            {#snippet control()}
              <DaysPicker
                value={schedule.days ?? []}
                onchange={setScheduleDays}
              />
            {/snippet}
          </Row>
        {/if}

        <Row label="Suppress when fullscreen" id="suppress-fullscreen">
          {#snippet control()}
            <Switch
              value={dnd.suppress_fullscreen ?? true}
              onchange={setSuppressFullscreen}
              ariaLabel="Suppress when fullscreen"
            />
          {/snippet}
        </Row>
      </Group>

      <!-- ── LISTS ────────────────────────────────── -->
      <Group label="Lists">
        <Row label="Always allow" id="always-allow">
          {#snippet control()}
            <div class="list-control">
              <AppPicker
                knownApps={knownAppsForPicker}
                excluded={alwaysAllow}
                placeholder="Add app..."
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
        <Row label="Always suppress" id="always-suppress">
          {#snippet control()}
            <div class="list-control">
              <AppPicker
                knownApps={knownAppsForPicker}
                excluded={alwaysSuppress}
                placeholder="Add app..."
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
      </Group>

      <!-- ── TOAST APPEARANCE ────────────────────────────────── -->
      <Group label="Toast Appearance">
        <Row label="Position" id="toast-position">
          {#snippet control()}
            <PositionPicker
              value={toast.position ?? "top-right"}
              onchange={setToastPosition}
            />
          {/snippet}
        </Row>
        <Row label="Width" id="toast-width">
          {#snippet control()}
            <ValueSlider
              value={toast.width ?? 380}
              min={300}
              max={500}
              step={10}
              unit="px"
              ariaLabel="Toast width"
              onchange={setToastWidth}
            />
          {/snippet}
        </Row>
        <Row label="Animation" id="toast-animation">
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
      </Group>

      <!-- ── TIMING ────────────────────────────────── -->
      <Group label="Timing">
        <Row label="Normal duration" id="toast-duration-normal">
          {#snippet control()}
            <ValueSlider
              value={general.toast_duration_normal ?? 4000}
              min={1000}
              max={15000}
              step={500}
              unit="ms"
              ariaLabel="Normal duration"
              onchange={(v) => setGeneral("toast_duration_normal", v)}
            />
          {/snippet}
        </Row>
        <Row label="High priority duration" id="toast-duration-high">
          {#snippet control()}
            <ValueSlider
              value={general.toast_duration_high ?? 8000}
              min={3000}
              max={30000}
              step={1000}
              unit="ms"
              ariaLabel="High priority duration"
              onchange={(v) => setGeneral("toast_duration_high", v)}
            />
          {/snippet}
        </Row>
        <Row label="Max visible" id="max-visible">
          {#snippet control()}
            <ValueSlider
              value={general.max_visible_toasts ?? 5}
              min={1}
              max={10}
              step={1}
              unit=""
              ariaLabel="Max visible toasts"
              onchange={(v) => setGeneral("max_visible_toasts", v)}
            />
          {/snippet}
        </Row>
        <Row label="Test notification">
          {#snippet control()}
            <div class="test-row">
              <button
                type="button"
                class="test-btn"
                onclick={() => fireTest("normal")}>Normal</button
              >
              <button
                type="button"
                class="test-btn"
                onclick={() => fireTest("high")}>High</button
              >
              <button
                type="button"
                class="test-btn danger"
                onclick={() => fireTest("critical")}>Critical</button
              >
            </div>
          {/snippet}
        </Row>
      </Group>

      <!-- ── GROUPING ────────────────────────────────── -->
      <Group label="Grouping">
        <Row label="Group by app" id="group-by-app">
          {#snippet control()}
            <Switch
              value={grouping.by_app ?? true}
              onchange={(v) => setGrouping("by_app", v)}
              ariaLabel="Group by app"
            />
          {/snippet}
        </Row>
        <Row label="Stack similar" id="stack-similar">
          {#snippet control()}
            <Switch
              value={grouping.stack_similar ?? true}
              onchange={(v) => setGrouping("stack_similar", v)}
              ariaLabel="Stack similar"
            />
          {/snippet}
        </Row>
        <Row label="Auto-collapse after" id="auto-collapse">
          {#snippet control()}
            <ValueSlider
              value={grouping.auto_collapse_after ?? 3}
              min={2}
              max={10}
              step={1}
              unit=""
              ariaLabel="Auto-collapse after"
              onchange={(v) => setGrouping("auto_collapse_after", v)}
            />
          {/snippet}
        </Row>
      </Group>

      <!-- ── HISTORY ────────────────────────────────── -->
      <Group label="History">
        <Row label="Keep history" id="history-enabled">
          {#snippet control()}
            <Switch
              value={history.enabled ?? true}
              onchange={(v) => setHistory("enabled", v)}
              ariaLabel="Keep history"
            />
          {/snippet}
        </Row>
        <Row label="Maximum age" id="history-max-age">
          {#snippet control()}
            <ValueSlider
              value={history.max_age_days ?? 30}
              min={1}
              max={90}
              step={1}
              unit=" days"
              ariaLabel="Maximum age"
              onchange={(v) => setHistory("max_age_days", v)}
            />
          {/snippet}
        </Row>
        <Row label="Maximum count" id="history-max-count">
          {#snippet control()}
            <ValueSlider
              value={history.max_count ?? 1000}
              min={100}
              max={5000}
              step={100}
              unit=""
              ariaLabel="Maximum count"
              onchange={(v) => setHistory("max_count", v)}
            />
          {/snippet}
        </Row>
        <Row label="Clear all history">
          {#snippet control()}
            <button
              type="button"
              class="danger-btn"
              class:confirming={confirmingClear}
              onclick={clearHistory}
            >
              <Trash2 size={12} strokeWidth={2.25} />
              {confirmingClear ? "Confirm — click again" : "Clear history"}
            </button>
          {/snippet}
        </Row>
      </Group>

      <!-- ── PER-APP ────────────────────────────────── -->
      <Group label="Per-App Rules">
        <div class="apps-section">
          <div class="apps-toolbar">
            <input
              type="text"
              class="apps-filter"
              placeholder="Filter rules..."
              value={appFilter}
              oninput={(e) =>
                (appFilter = (e.currentTarget as HTMLInputElement).value)}
            />
            <AppPicker
              knownApps={knownAppsForPicker}
              excluded={appNames}
              placeholder="Add rule for app..."
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
      </Group>
    </div>
  {/if}
</div>

<style>
  .page {
    width: 100%;
    max-width: 44rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .head {
    margin-bottom: 1.25rem;
  }
  h1 {
    margin: 0 0 0.25rem;
    font-size: 1.125rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--foreground);
  }
  .lede {
    margin: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .groups {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .status {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .error {
    padding: 0.75rem 1rem;
    border-radius: var(--radius-md);
    border: 1px solid color-mix(in srgb, var(--color-error) 40%, transparent);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    color: var(--color-error);
    font-size: 0.8125rem;
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
    text-align: left;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    cursor: pointer;
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
    border-radius: var(--radius-sm);
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .dnd-pill.active .dnd-pill-icon {
    color: var(--color-accent);
  }
  .dnd-pill-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .dnd-pill-hint {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    line-height: 1.25;
  }

  .expires-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--color-accent);
    font-size: 0.75rem;
  }
  .expires-banner strong {
    color: var(--foreground);
    font-weight: 600;
  }
  .expires-banner .link {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--color-accent);
    font: inherit;
    font-size: 0.6875rem;
    text-decoration: underline;
    cursor: pointer;
    padding: 0;
  }

  .quick-actions {
    display: flex;
    gap: 6px;
  }
  .quick-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0.4rem 0.65rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    font-size: 0.75rem;
    cursor: pointer;
    transition: background-color 120ms ease;
  }
  .quick-btn:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
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
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--foreground);
    font-size: 0.6875rem;
    cursor: pointer;
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

  /* ── Segmented ────────────────────────────────── */
  .seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
  }
  .seg-pill {
    height: 22px;
    padding: 0 0.6rem;
    border-radius: var(--radius-sm);
    background: transparent;
    border: none;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.6875rem;
    font-weight: 500;
    cursor: pointer;
    text-transform: capitalize;
    transition: all 120ms ease;
  }
  .seg-pill:hover {
    color: var(--foreground);
  }
  .seg-pill.active {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--foreground);
  }

  /* ── Test buttons ────────────────────────────────── */
  .test-row {
    display: flex;
    gap: 6px;
  }
  .test-btn {
    height: 26px;
    padding: 0 0.65rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    font-size: 0.6875rem;
    cursor: pointer;
    transition: background-color 120ms ease;
  }
  .test-btn:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }
  .test-btn.danger {
    color: var(--color-error);
    border-color: color-mix(in srgb, var(--color-error) 35%, transparent);
  }
  .test-btn.danger:hover {
    background: color-mix(in srgb, var(--color-error) 14%, transparent);
  }

  /* ── Danger button ────────────────────────────────── */
  .danger-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0.4rem 0.65rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-error) 35%, transparent);
    color: var(--color-error);
    font-size: 0.75rem;
    cursor: pointer;
    transition:
      background-color 120ms ease,
      border-color 120ms ease;
  }
  .danger-btn:hover {
    background: color-mix(in srgb, var(--color-error) 18%, transparent);
  }
  .danger-btn.confirming {
    background: color-mix(in srgb, var(--color-error) 28%, transparent);
    border-color: var(--color-error);
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
  .apps-filter {
    height: 28px;
    padding: 0 0.6rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    font: inherit;
    font-size: 0.75rem;
    outline: none;
  }
  .apps-filter:focus-visible {
    border-color: color-mix(in srgb, var(--color-accent) 40%, transparent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 18%, transparent);
  }
  .apps-empty {
    padding: 0.75rem 0.6rem;
    text-align: center;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .apps-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
