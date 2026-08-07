<script lang="ts">
  /// Physical privacy (privacy-sentinel-plan.md §7): the sentinel's Settings
  /// surface. Two sections carry the trust story - the deterministic protections
  /// are on by default, the ambient watchers are opt-in - and every card says in
  /// plain language what its detector cannot see. The honest framing is the
  /// product; nothing here ever claims "you are safe".
  import { onMount } from "svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { t } from "$lib/i18n/messages";
  import {
    sentinel,
    sentinelMocked,
    loadSentinel,
    setDetector,
    setAlerts,
    setSensitivity,
    fixPosture,
    type AlertMode,
    type DetectorId,
  } from "$lib/stores/sentinel";
  import { sensing, sensingUnknown, loadSensing, setScreenCapture } from "$lib/stores/sensing";

  onMount(() => {
    void loadSentinel();
    void loadSensing();
  });

  const ALERT_OPTIONS = $derived([
    { value: "quiet", label: $t("s.sent.alerts.quiet") },
    { value: "notify", label: $t("s.sent.alerts.notify") },
  ]);
  const PROXIMITY_OPTIONS = $derived([
    { value: "near", label: $t("s.sent.prox.near") },
    { value: "room", label: $t("s.sent.prox.room") },
    { value: "anywhere", label: $t("s.sent.prox.anywhere") },
  ]);
  const STRICTNESS_OPTIONS = $derived([
    { value: "cautious", label: $t("s.sent.strict.cautious") },
    { value: "balanced", label: $t("s.sent.strict.balanced") },
    { value: "strict", label: $t("s.sent.strict.strict") },
  ]);

  // Turning an always-on protection OFF is a rung-1 acknowledge: the dialog
  // names the exact consequence; cancel leaves it on. The kit Switch keeps its
  // own flipped state after a click, so a cancel re-mounts the toggles to snap
  // them back to the store's truth.
  let pendingOff = $state<DetectorId | null>(null);
  let toggleEpoch = $state(0);
  function requestOff(id: DetectorId, on: boolean) {
    if (!on && (id === "usb" || id === "exposure")) {
      pendingOff = id;
      return;
    }
    void setDetector(id, on);
  }
  function confirmOff() {
    const id = pendingOff;
    pendingOff = null;
    if (id) void setDetector(id, false);
  }
  function cancelOff() {
    pendingOff = null;
    toggleEpoch += 1;
  }

  // Switching capture back ON is the heavy direction, and the asymmetry is this
  // page's job: the backend command says so, and a backend that refused without
  // a token would be inventing a second consent mechanism beside the one the
  // system has. Off is one click; on asks, because it restores capture to
  // everyone already holding a grant - the marker on those grant lines in the
  // App access list is the list of who.
  let pendingCapture = $state(false);
  function requestCapture(on: boolean) {
    if (on) {
      pendingCapture = true;
      return;
    }
    void setScreenCapture(false);
  }
  function confirmCapture() {
    pendingCapture = false;
    void setScreenCapture(true);
  }
  function cancelCapture() {
    pendingCapture = false;
    toggleEpoch += 1;
  }
</script>

<Page title={$t("s.sent.title")} description={$t("s.sent.desc")}>
  <SectionGrid>
    {#if $sentinelMocked}
      <p class="sample span-full">{$t("s.sent.sample")}</p>
    {/if}

    {#if $sentinel}
      {@const d = $sentinel.detectors}

      <div class="section-label span-full">{$t("s.sent.alwaysOn")}</div>

      <Section label={$t("s.sent.exposure")} class="span-full">
        <Row label={$t("s.sent.exposure.row")} description={$t("s.sent.exposure.rowDesc")} id="sent-exposure">
          {#snippet control()}
            {#key toggleEpoch}
              <Switch value={d.exposure.on} ariaLabel={$t("s.sent.exposure.row")} onchange={(v) => requestOff("exposure", v)} />
            {/key}
          {/snippet}
        </Row>
        {#if d.exposure.on}
          <div class="posture">
            {#each $sentinel.posture as line, i (line.text)}
              <div class="posture-line" class:warn={line.fix}>
                <span>{line.text}</span>
                {#if line.fix}
                  <Button variant="outline" size="sm" id="sent-exposure-fix" onclick={() => fixPosture(i)}>
                    {$t("s.sent.fix")}
                  </Button>
                {/if}
              </div>
            {/each}
          </div>
          <Row label={$t("s.sent.alerts")} id="sent-exposure-alerts">
            {#snippet control()}
              <SegmentedControl value={d.exposure.alerts} options={ALERT_OPTIONS} ariaLabel={$t("s.sent.alerts")} onchange={(v) => setAlerts("exposure", v as AlertMode)} />
            {/snippet}
          </Row>
        {/if}
        <p class="caveat">{$t("s.sent.exposure.caveat")}</p>
      </Section>

      <!-- The one switch here that refuses rather than reports. The sections
           around it say what a detector noticed; this says what the system will
           not do, and it is enforced by the portal and the compositor rather
           than by this page. -->
      <Section label={$t("s.sens.screen")} class="span-full">
        <Row
          label={$t("s.sens.screen.row")}
          description={$t("s.sens.screen.rowDesc")}
          id="sens-screen-capture"
        >
          {#snippet control()}
            {#key toggleEpoch}
              <Switch
                value={$sensing.screenCapture}
                onchange={(v) => requestCapture(v)}
                ariaLabel={$t("s.sens.screen.row")}
              />
            {/key}
          {/snippet}
        </Row>
        {#if !$sensing.screenCapture}
          <p class="caveat">{$t("s.sens.screen.off")}</p>
        {/if}
        {#if $sensingUnknown}
          <p class="caveat">{$t("s.sens.unknown")}</p>
        {/if}
      </Section>

      <Section label={$t("s.sent.capture")} class="span-full">
        <p class="status">
          {$sentinel.captureActive ? $t("s.sent.capture.active") : $t("s.sent.capture.idle")}
        </p>
        <p class="caveat">{$t("s.sent.capture.caveat")}</p>
      </Section>

      <Section label={$t("s.sent.usb")} class="span-full">
        <Row label={$t("s.sent.usb.row")} description={$t("s.sent.usb.rowDesc")} id="sent-usb">
          {#snippet control()}
            {#key toggleEpoch}
              <Switch value={d.usb.on} ariaLabel={$t("s.sent.usb.row")} onchange={(v) => requestOff("usb", v)} />
            {/key}
          {/snippet}
        </Row>
        {#if d.usb.on}
          <Row label={$t("s.sent.alerts")} id="sent-usb-alerts">
            {#snippet control()}
              <SegmentedControl value={d.usb.alerts} options={ALERT_OPTIONS} ariaLabel={$t("s.sent.alerts")} onchange={(v) => setAlerts("usb", v as AlertMode)} />
            {/snippet}
          </Row>
        {/if}
        <p class="caveat">{$t("s.sent.usb.caveat")}</p>
      </Section>

      <div class="section-label span-full">{$t("s.sent.optIn")}</div>

      <Section label={$t("s.sent.recording")} class="span-full">
        <Row label={$t("s.sent.recording.row")} description={$t("s.sent.recording.rowDesc")} id="sent-recording">
          {#snippet control()}
            <Switch value={d.recording.on} ariaLabel={$t("s.sent.recording.row")} onchange={(v) => setDetector("recording", v)} />
          {/snippet}
        </Row>
        {#if d.recording.on}
          <Row label={$t("s.sent.alerts")} description={$t("s.sent.recording.quietOnly")} id="sent-recording-alerts">
            {#snippet control()}
              <SegmentedControl value="quiet" options={ALERT_OPTIONS} ariaLabel={$t("s.sent.alerts")} disabled />
            {/snippet}
          </Row>
          <Row label={$t("s.sent.sensitivity")} id="sent-recording-sensitivity">
            {#snippet control()}
              <SegmentedControl value={d.recording.sensitivity ?? "room"} options={PROXIMITY_OPTIONS} ariaLabel={$t("s.sent.sensitivity")} onchange={(v) => setSensitivity("recording", v)} />
            {/snippet}
          </Row>
        {/if}
        <p class="caveat">{$t("s.sent.recording.caveat")}</p>
      </Section>

      <Section label={$t("s.sent.tracker")} class="span-full">
        <Row label={$t("s.sent.tracker.row")} description={$t("s.sent.tracker.rowDesc")} id="sent-tracker">
          {#snippet control()}
            <Switch
              value={d.tracker.on}
              ariaLabel={$t("s.sent.tracker.row")}
              disabled={!$sentinel.trackerHasLocation}
              onchange={(v) => setDetector("tracker", v)}
            />
          {/snippet}
        </Row>
        {#if !$sentinel.trackerHasLocation}
          <p class="status">{$t("s.sent.tracker.needsLocation")}</p>
        {:else if d.tracker.on}
          <Row label={$t("s.sent.alerts")} id="sent-tracker-alerts">
            {#snippet control()}
              <SegmentedControl value={d.tracker.alerts} options={ALERT_OPTIONS} ariaLabel={$t("s.sent.alerts")} onchange={(v) => setAlerts("tracker", v as AlertMode)} />
            {/snippet}
          </Row>
          <Row label={$t("s.sent.sensitivity")} id="sent-tracker-sensitivity">
            {#snippet control()}
              <SegmentedControl value={d.tracker.sensitivity ?? "balanced"} options={STRICTNESS_OPTIONS} ariaLabel={$t("s.sent.sensitivity")} onchange={(v) => setSensitivity("tracker", v)} />
            {/snippet}
          </Row>
        {/if}
        <p class="caveat">{$t("s.sent.tracker.caveat")}</p>
      </Section>
    {/if}
  </SectionGrid>
</Page>

<ConfirmDialog
  open={pendingCapture}
  title={$t("s.sens.screen.onTitle")}
  message={$t("s.sens.screen.onMsg")}
  confirmLabel={$t("s.sens.screen.turnOn")}
  onConfirm={confirmCapture}
  onCancel={cancelCapture}
/>

<ConfirmDialog
  open={pendingOff !== null}
  title={pendingOff === "usb" ? $t("s.sent.usb.offTitle") : $t("s.sent.exposure.offTitle")}
  message={pendingOff === "usb" ? $t("s.sent.usb.offMsg") : $t("s.sent.exposure.offMsg")}
  confirmLabel={$t("s.sent.turnOff")}
  variant="destructive"
  onConfirm={confirmOff}
  onCancel={cancelOff}
/>

<style>
  .sample {
    margin: 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .section-label {
    padding: 0.5rem 0.25rem 0;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  /* The exposure posture readout: one prose line per surface; the line that
     needs attention carries its one-click fix. */
  .posture {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.25rem 1rem 0.5rem;
  }
  .posture-line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .posture-line.warn {
    color: var(--foreground);
  }

  .status {
    margin: 0;
    padding: 0.6rem 1rem 0.25rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }

  /* The honest limit, on every card - inline, never a footnote. */
  .caveat {
    margin: 0;
    padding: 0.4rem 1rem 0.75rem;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
