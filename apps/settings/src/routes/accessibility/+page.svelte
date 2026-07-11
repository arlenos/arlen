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
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { t } from "$lib/i18n/messages";
  import {
    compositor,
    screenFilter,
    loadFilter,
    setInverted,
    setColorFilter,
    ZOOM_DEFAULTS,
    ZOOM_MOVEMENT_OPTIONS,
    COLOR_FILTER_OPTIONS,
    type ZoomMovement,
    type ColorFilterLabel,
  } from "$lib/stores/accessibility";

  onMount(() => {
    compositor.load();
    loadFilter();
  });

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
  <SectionGrid>
    <Group label={$t("s.a11y.magnifier")}>
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
          options={ZOOM_MOVEMENT_OPTIONS as unknown as { value: string; label: string }[]}
          ariaLabel={$t("s.a11y.movement.aria")}
          width="180px"
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
  </Group>

  <Group label={$t("s.a11y.colorFilters")}>
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
          options={COLOR_FILTER_OPTIONS as unknown as { value: string; label: string }[]}
          ariaLabel={$t("s.a11y.colorBlind")}
          width="240px"
          onchange={(v) => setColorFilter(v as ColorFilterLabel)}
        />
      {/snippet}
    </Row>
    </Group>
  </SectionGrid>
</Page>
