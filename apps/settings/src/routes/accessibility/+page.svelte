<script lang="ts">
  /// Accessibility settings page (Sprint C).
  ///
  /// Magnifier settings → `compositor.toml [accessibility_zoom]`
  /// (live-reload via the compositor's existing watcher).
  /// Color filter + invert → state file
  /// `~/.local/state/cosmic-comp/a11y_screen_filter.ron` via the
  /// `accessibility_filter_set/get` Tauri commands; the compositor's
  /// notify-watcher applies the change within ~100 ms.

  import { onMount } from "svelte";
  import ConfigWriteFailed from "$lib/components/ConfigWriteFailed.svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { formatDecimal } from "@arlen/ui-kit/i18n";
  import { t, locale } from "$lib/i18n/messages";
  import {
    contrastReport,
    contrastUnavailable,
    loadContrast,
    failing,
    type ContrastRole,
  } from "$lib/stores/contrast";
  import {
    compositor,
    screenFilter,
    loadFilter,
    setInverted,
    setColorFilter,
    ZOOM_DEFAULTS,
    zoomMovementOptions,
    colorFilterOptions,
    type ZoomMovement,
    type ColorFilterLabel,
  } from "$lib/stores/accessibility";

  onMount(() => {
    compositor.load();
    loadFilter();
    loadContrast();
  });

  /// The sentence for one failing pair.
  ///
  /// Which floor it is held to arrives as a translated word inside the sentence
  /// rather than as a second sentence beside it, so a German reading is one
  /// clause and not two glued together.
  function missText(r: ContrastRole): string {
    const floor = $t(r.usage === "large" ? "s.a11y.contrastFloorLarge" : "s.a11y.contrastFloorBody");
    const wcag = formatDecimal(r.wcag, 1, $locale);
    const apca = formatDecimal(Math.abs(r.apca), 0, $locale);
    if (!r.wcagPass && !r.apcaPass) return $t("s.a11y.contrastMissBoth", { floor, wcag, apca });
    if (!r.wcagPass) return $t("s.a11y.contrastMissWcag", { floor, wcag });
    return $t("s.a11y.contrastMissApca", { floor, apca });
  }

  // Derived current values with defaults filled in.
  const enableMouseZoom = $derived<boolean>(
    ($compositor.data?.accessibility_zoom?.enable_mouse_zoom_shortcuts as
      | boolean
      | undefined) ?? ZOOM_DEFAULTS.enable_mouse_zoom_shortcuts,
  );
  const increment = $derived<number>(
    ($compositor.data?.accessibility_zoom?.increment as number | undefined) ??
      ZOOM_DEFAULTS.increment,
  );
  const viewMoves = $derived<ZoomMovement>(
    ($compositor.data?.accessibility_zoom?.view_moves as
      | ZoomMovement
      | undefined) ?? ZOOM_DEFAULTS.view_moves,
  );
  const showOverlay = $derived<boolean>(
    ($compositor.data?.accessibility_zoom?.show_overlay as
      | boolean
      | undefined) ?? ZOOM_DEFAULTS.show_overlay,
  );
  const startOnLogin = $derived<boolean>(
    ($compositor.data?.accessibility_zoom?.start_on_login as
      | boolean
      | undefined) ?? ZOOM_DEFAULTS.start_on_login,
  );

  const inverted = $derived<boolean>($screenFilter.data.inverted);
  const colorFilter = $derived<ColorFilterLabel>(
    ($screenFilter.data.colorFilter as ColorFilterLabel | null) ?? "None",
  );

  async function setEnableMouseZoom(v: boolean) {
    await compositor.setValue("accessibility_zoom.enable_mouse_zoom_shortcuts", v);
  }
  async function setIncrement(v: number) {
    await compositor.setValue("accessibility_zoom.increment", v);
  }
  async function setViewMoves(v: string) {
    await compositor.setValue("accessibility_zoom.view_moves", v);
  }
  async function setShowOverlay(v: boolean) {
    await compositor.setValue("accessibility_zoom.show_overlay", v);
  }
  async function setStartOnLogin(v: boolean) {
    await compositor.setValue("accessibility_zoom.start_on_login", v);
  }
</script>

<Page
  title={$t("s.a11y.title")}
  description={$t("s.a11y.desc")}
>
  <!-- Two reads back this page, and the screen filter is the one that is not in
       compositor.toml: it comes from its own command with its own failure. The
       first version of this line named only the compositor store, so a filter
       read that failed left the inverted and colour-filter rows showing defaults
       with nothing said. -->
  <ConfigWriteFailed failed={$compositor.writeFailed || $screenFilter.writeFailed} />
  <SectionGrid>
    {#if $compositor.error ?? $screenFilter.error}
      <Notice tone="error" class="span-full" text={$t("s.config.unavailable")} />
    {/if}
    <fieldset class="unreadable-gate" disabled={($compositor.error ?? $screenFilter.error) !== null}>
    <Section label={$t("s.a11y.magnifier")}>
    <Row
      label={$t("s.a11y.mouseZoom")}
      description={$t("s.a11y.mouseZoom.desc")}
      id="zoom-shortcuts"
    >
      {#snippet control()}
        <Switch
          value={enableMouseZoom}
          ariaLabel={$t("s.a11y.mouseZoom")}
          onchange={setEnableMouseZoom}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.a11y.increment")}
      description={$t("s.a11y.increment.desc")}
      id="zoom-increment"
    >
      {#snippet control()}
        <ValueSlider
          value={increment}
          min={5}
          max={200}
          step={5}
          unit="%"
          ariaLabel={$t("s.a11y.increment")}
          onchange={setIncrement}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.a11y.movement")}
      description={$t("s.a11y.movement.desc")}
      id="zoom-movement"
    >
      {#snippet control()}
        <PopoverSelect
          value={viewMoves}
          options={$zoomMovementOptions as unknown as { value: string; label: string }[]}
          ariaLabel={$t("s.a11y.movement.aria")}
          onchange={setViewMoves}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.a11y.overlay")}
      description={$t("s.a11y.overlay.desc")}
      id="zoom-overlay"
    >
      {#snippet control()}
        <Switch
          value={showOverlay}
          ariaLabel={$t("s.a11y.overlay")}
          onchange={setShowOverlay}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.a11y.startLogin")}
      description={$t("s.a11y.startLogin.desc")}
      id="zoom-start-on-login"
    >
      {#snippet control()}
        <Switch
          value={startOnLogin}
          ariaLabel={$t("s.a11y.startLogin")}
          onchange={setStartOnLogin}
        />
      {/snippet}
    </Row>
  </Section>

  <Section label={$t("s.a11y.colorFilters")}>
    <Row
      label={$t("s.a11y.invert")}
      description={$t("s.a11y.invert.desc")}
      id="invert-colors"
    >
      {#snippet control()}
        <Switch
          value={inverted}
          ariaLabel={$t("s.a11y.invert")}
          onchange={setInverted}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.a11y.colorBlind")}
      description={$t("s.a11y.colorBlind.desc")}
      id="color-blindness-filter"
    >
      {#snippet control()}
        <PopoverSelect
          value={colorFilter}
          options={$colorFilterOptions as unknown as { value: string; label: string }[]}
          ariaLabel={$t("s.a11y.colorBlind")}
          onchange={(v) => setColorFilter(v as ColorFilterLabel)}
        />
      {/snippet}
    </Row>
    </Section>

  <!-- Measured, not claimed. The audit runs over the RESOLVED theme, so a
       custom appearance or an accent somebody picked is judged the same way a
       shipped one is - which is the case that actually goes wrong. -->
  <Section label={$t("s.a11y.contrast")}>
    {#if $contrastUnavailable}
      <p class="contrast-note">{$t("s.a11y.contrastUnavailable")}</p>
    {:else if $contrastReport === null}
      <p class="contrast-note">{$t("s.a11y.contrastReading")}</p>
    {:else if failing($contrastReport).length === 0}
      <p class="contrast-note">{$t("s.a11y.contrastAllPass")}</p>
    {:else}
      <p class="contrast-note" role="alert">{$t("s.a11y.contrastSomeFail")}</p>
      {#each failing($contrastReport) as r (r.pair)}
        <Row label={r.pair} description={missText(r)} />
      {/each}
    {/if}
  </Section>
    </fieldset>
  </SectionGrid>
</Page>

<style>
  /* 55%: this line is often the only thing in its section, so it is held to the
     readable floor rather than the decorative one. */
  .contrast-note {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
</style>
