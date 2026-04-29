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
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Switch } from "$lib/components/ui/switch";
  import { ValueSlider } from "$lib/components/ui/value-slider";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
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

<SettingsPage
  title="Accessibility"
  description="Screen magnifier, color filters, and visual aids."
>
  <Group label="Screen Magnifier">
    <Row
      label="Enable mouse zoom shortcuts"
      description="Super+Scroll, Super+= and Super+- to zoom in and out."
      id="zoom-shortcuts"
    >
      {#snippet control()}
        <Switch
          value={enableMouseZoom}
          ariaLabel="Enable mouse zoom shortcuts"
          onchange={setEnableMouseZoom}
        />
      {/snippet}
    </Row>

    <Row
      label="Zoom increment"
      description="How much each zoom step changes the magnification."
      id="zoom-increment"
    >
      {#snippet control()}
        <ValueSlider
          value={increment}
          min={5}
          max={200}
          step={5}
          unit="%"
          ariaLabel="Zoom increment"
          onchange={setIncrement}
        />
      {/snippet}
    </Row>

    <Row
      label="Movement"
      description="How the magnified region tracks the mouse cursor."
      id="zoom-movement"
    >
      {#snippet control()}
        <PopoverSelect
          value={viewMoves}
          options={ZOOM_MOVEMENT_OPTIONS as unknown as { value: string; label: string }[]}
          ariaLabel="Zoom movement"
          width="180px"
          onchange={setViewMoves}
        />
      {/snippet}
    </Row>

    <Row
      label="Show zoom overlay"
      description="Outline indicating the magnified region."
      id="zoom-overlay"
    >
      {#snippet control()}
        <Switch
          value={showOverlay}
          ariaLabel="Show zoom overlay"
          onchange={setShowOverlay}
        />
      {/snippet}
    </Row>

    <Row
      label="Start zoom on login"
      description="Auto-enable the magnifier when the session starts. Takes effect on next login."
      id="zoom-start-on-login"
    >
      {#snippet control()}
        <Switch
          value={startOnLogin}
          ariaLabel="Start zoom on login"
          onchange={setStartOnLogin}
        />
      {/snippet}
    </Row>
  </Group>

  <Group label="Color Filters">
    <Row
      label="Invert colors"
      description="High-contrast inverted display. Helpful in dark environments."
      id="invert-colors"
    >
      {#snippet control()}
        <Switch
          value={inverted}
          ariaLabel="Invert colors"
          onchange={setInverted}
        />
      {/snippet}
    </Row>

    <Row
      label="Color blindness filter"
      description="Compensation for the most common forms of color blindness."
      id="color-blindness-filter"
    >
      {#snippet control()}
        <PopoverSelect
          value={colorFilter}
          options={COLOR_FILTER_OPTIONS as unknown as { value: string; label: string }[]}
          ariaLabel="Color blindness filter"
          width="240px"
          onchange={(v) => setColorFilter(v as ColorFilterLabel)}
        />
      {/snippet}
    </Row>
  </Group>
</SettingsPage>
