<script lang="ts">
  /// Appearance panel. Compact settings-app layout: one screen, no
  /// scroll, grouped cards with divided rows.

  import { onMount } from "svelte";
  import {
    theme,
    FONT_OPTIONS,
    MONO_FONT_OPTIONS,
    resolveAccent,
    BORDER_ACCENT_SENTINEL,
    BORDER_SUBTLE_SENTINEL,
    type ThemeMode,
  } from "$lib/stores/theme";
  import { compositor } from "$lib/stores/compositor";

  import Group from "$lib/components/appearance/Group.svelte";
  import Row from "$lib/components/appearance/Row.svelte";
  import ModeToggle from "$lib/components/appearance/ModeToggle.svelte";
  import AccentPicker from "$lib/components/appearance/AccentPicker.svelte";
  import ValueSlider from "$lib/components/appearance/ValueSlider.svelte";
  import FontSelect from "$lib/components/appearance/FontSelect.svelte";
  import Switch from "$lib/components/appearance/Switch.svelte";
  import BorderColorPicker from "$lib/components/appearance/BorderColorPicker.svelte";

  onMount(() => {
    theme.load();
    compositor.load();
  });

  // ── Derived values ────────────────────────────────────────────────────

  const currentMode = $derived.by((): ThemeMode => {
    const s = $theme.data;
    if (!s) return "dark";
    if (s.theme.mode) return s.theme.mode;
    return (s.theme.active as ThemeMode) ?? "dark";
  });

  const currentAccent = $derived(resolveAccent($theme.data));
  const accentOverride = $derived($theme.data?.overrides?.accent);
  const cornerRadius = $derived($theme.data?.window?.corner_radius ?? 8);
  const borderWidth = $derived($theme.data?.window?.border_width ?? 2);
  // Gaps live in compositor.toml [layout], not appearance.toml.
  // Same path the desktop-shell LayoutPopover writes to, so the
  // existing compositor watcher picks them up automatically.
  const gaps = $derived($compositor.data?.layout?.inner_gap ?? 8);
  const gapSmart = $derived($compositor.data?.layout?.smart_gaps ?? true);
  const borderFocused = $derived(
    $theme.data?.window?.border?.focused ?? BORDER_ACCENT_SENTINEL
  );
  const borderUnfocused = $derived(
    $theme.data?.window?.border?.unfocused ?? BORDER_SUBTLE_SENTINEL
  );
  const interfaceFont = $derived(
    $theme.data?.fonts?.interface ?? "Inter Variable"
  );
  const monospaceFont = $derived(
    $theme.data?.fonts?.monospace ?? "JetBrains Mono"
  );
  const fontSize = $derived($theme.data?.fonts?.size ?? 14);

  // ── Setters ───────────────────────────────────────────────────────────

  async function setMode(mode: ThemeMode) {
    await theme.setValue("theme.mode", mode);
    await theme.setValue("theme.active", mode);
  }

  async function setAccent(hex: string) {
    await theme.setValue("overrides.accent", hex);
  }

  async function setWindow(key: string, value: number | boolean) {
    await theme.setValue(`window.${key}`, value);
  }

  async function setBorderColor(side: "focused" | "unfocused", value: string) {
    await theme.setValue(`window.border.${side}`, value);
  }

  /// Single slider: inner == outer, same pattern as the shell LayoutPopover.
  async function setGaps(value: number) {
    await compositor.setValue("layout.inner_gap", value);
    await compositor.setValue("layout.outer_gap", value);
  }

  async function setSmartGaps(enabled: boolean) {
    await compositor.setValue("layout.smart_gaps", enabled);
  }

  async function setFont(key: string, value: string | number) {
    await theme.setValue(`fonts.${key}`, value);
  }
</script>

<div class="page">
  <header class="head">
    <h1>Appearance</h1>
  </header>

  {#if $theme.loading && !$theme.data}
    <div class="status">Loading…</div>
  {:else if $theme.error && !$theme.data}
    <div class="error">
      Failed to load appearance config: {$theme.error}
    </div>
  {:else}
    <div class="groups">
      <Group label="Theme">
        <Row label="Mode">
          {#snippet control()}
            <ModeToggle value={currentMode} onchange={setMode} />
          {/snippet}
        </Row>
        <Row label="Accent">
          {#snippet control()}
            <AccentPicker
              value={currentAccent}
              rawOverride={accentOverride}
              onchange={setAccent}
            />
          {/snippet}
        </Row>
      </Group>

      <Group label="Window">
        <Row label="Corner Radius">
          {#snippet preview()}
            <div
              class="radius-preview"
              style="border-radius: {cornerRadius}px;"
            ></div>
          {/snippet}
          {#snippet control()}
            <ValueSlider
              value={cornerRadius}
              min={0}
              max={16}
              step={1}
              unit="px"
              ariaLabel="Corner Radius"
              onchange={(v) => setWindow("corner_radius", v)}
            />
          {/snippet}
        </Row>
        <Row label="Border Width">
          {#snippet preview()}
            <div
              class="border-preview"
              style="border-width: {Math.max(borderWidth, 1)}px; opacity: {borderWidth === 0 ? 0.3 : 1};"
            ></div>
          {/snippet}
          {#snippet control()}
            <ValueSlider
              value={borderWidth}
              min={0}
              max={4}
              step={1}
              unit="px"
              ariaLabel="Border Width"
              onchange={(v) => setWindow("border_width", v)}
            />
          {/snippet}
        </Row>
        <Row label="Gaps">
          {#snippet preview()}
            <div class="gap-preview" style="gap: {Math.min(gaps, 6)}px;">
              <span></span>
              <span></span>
            </div>
          {/snippet}
          {#snippet control()}
            <ValueSlider
              value={gaps}
              min={0}
              max={24}
              step={1}
              unit="px"
              ariaLabel="Gaps"
              onchange={setGaps}
            />
          {/snippet}
        </Row>
        <Row label="Smart Gaps">
          {#snippet control()}
            <Switch
              value={gapSmart}
              ariaLabel="Smart gaps"
              onchange={setSmartGaps}
            />
          {/snippet}
        </Row>
      </Group>

      <Group label="Window Borders">
        <Row label="Focused">
          {#snippet control()}
            <BorderColorPicker
              value={borderFocused}
              sentinel={BORDER_ACCENT_SENTINEL}
              sentinelLabel="Accent"
              sentinelSwatch={currentAccent}
              onchange={(v) => setBorderColor("focused", v)}
            />
          {/snippet}
        </Row>
        <Row label="Unfocused">
          {#snippet control()}
            <BorderColorPicker
              value={borderUnfocused}
              sentinel={BORDER_SUBTLE_SENTINEL}
              sentinelLabel="Subtle"
              sentinelSwatch="color-mix(in srgb, var(--foreground) 15%, transparent)"
              onchange={(v) => setBorderColor("unfocused", v)}
            />
          {/snippet}
        </Row>
      </Group>

      <Group label="Typography">
        <Row label="Interface">
          {#snippet control()}
            <FontSelect
              value={interfaceFont}
              options={FONT_OPTIONS}
              ariaLabel="Interface font"
              onchange={(v) => setFont("interface", v)}
            />
          {/snippet}
        </Row>
        <Row label="Monospace">
          {#snippet control()}
            <FontSelect
              value={monospaceFont}
              options={MONO_FONT_OPTIONS}
              ariaLabel="Monospace font"
              onchange={(v) => setFont("monospace", v)}
            />
          {/snippet}
        </Row>
        <Row label="Size">
          {#snippet preview()}
            <span
              class="size-preview"
              style="font-size: {fontSize}px;"
            >
              Aa
            </span>
          {/snippet}
          {#snippet control()}
            <ValueSlider
              value={fontSize}
              min={12}
              max={18}
              step={1}
              unit="px"
              ariaLabel="Font size"
              onchange={(v) => setFont("size", v)}
            />
          {/snippet}
        </Row>
      </Group>
    </div>
  {/if}
</div>

<style>
  .page {
    width: 100%;
    max-width: 42rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }

  .head {
    margin-bottom: 1.25rem;
  }

  h1 {
    margin: 0;
    font-size: 1.125rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--foreground);
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
    border-radius: 8px;
    border: 1px solid
      color-mix(in srgb, var(--color-error) 40%, transparent);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    color: var(--color-error);
    font-size: 0.8125rem;
  }

  /* ── Inline previews ─────────────────────────────── */
  .radius-preview {
    width: 22px;
    height: 22px;
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 20%, transparent);
    transition: border-radius 150ms ease;
  }

  .border-preview {
    width: 22px;
    height: 22px;
    border-radius: 4px;
    border-style: solid;
    border-color: color-mix(in srgb, var(--foreground) 35%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    transition: border-width 150ms ease, opacity 150ms ease;
  }

  .gap-preview {
    display: flex;
    align-items: center;
    height: 22px;
    transition: gap 150ms ease;
  }
  .gap-preview span {
    display: block;
    width: 9px;
    height: 22px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 20%, transparent);
  }

  .size-preview {
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    line-height: 1;
    font-variant-numeric: tabular-nums;
    transition: font-size 150ms ease;
  }
</style>
