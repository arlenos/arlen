<script lang="ts">
  /// Lunaris Color Picker (Tier 1: HSV pad + hue slider + hex input).
  ///
  /// Replaces the native `<input type="color">` which on Wayland +
  /// Tauri opens GTK's stock chooser dialog — visually inconsistent
  /// with the rest of the shell. This component is themed via the
  /// existing CSS variables (`--color-bg-card`, `--color-fg-shell`,
  /// `--radius-md` etc.) so it inherits the active theme.
  ///
  /// Bound state: `value` is a `#RRGGBB` string. Two-way bound; the
  /// caller can both seed the picker and observe changes.
  ///
  /// Out of scope (deferred follow-ups): alpha channel, saved-colors
  /// row, eye-dropper API, gradient picker. The color-picker pad +
  /// hue-slider + hex input is the minimum-viable replacement;
  /// everything else lands when a real consumer asks for it.

  interface Props {
    value: string;
    onchange?: (hex: string) => void;
  }

  let { value = $bindable("#6366f1"), onchange }: Props = $props();

  // Internal HSV state. Synced to/from `value` (hex) on each change.
  let h = $state(0); // 0..360
  let s = $state(1); // 0..1
  let v = $state(1); // 0..1
  let hexInput = $state(value);
  let suspendSync = false;

  // Reflect external `value` changes back into HSV (and the input
  // text). The `suspendSync` guard prevents this effect from
  // looping when WE are the source of the change.
  $effect(() => {
    if (suspendSync) return;
    const hsv = hexToHsv(value);
    if (hsv) {
      h = hsv.h;
      s = hsv.s;
      v = hsv.v;
      hexInput = value.toUpperCase();
    }
  });

  function commitHsv(newH: number, newS: number, newV: number) {
    h = newH;
    s = newS;
    v = newV;
    const hex = hsvToHex(newH, newS, newV);
    suspendSync = true;
    value = hex;
    hexInput = hex.toUpperCase();
    onchange?.(hex);
    queueMicrotask(() => {
      suspendSync = false;
    });
  }

  // ---- HSV pad: drag picker over a saturation × value rectangle.
  let padEl = $state<HTMLDivElement | null>(null);
  let padDragging = $state(false);

  function handlePadPointer(e: PointerEvent) {
    if (!padEl) return;
    const r = padEl.getBoundingClientRect();
    const px = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    const py = Math.max(0, Math.min(1, (e.clientY - r.top) / r.height));
    commitHsv(h, px, 1 - py);
  }

  function onPadDown(e: PointerEvent) {
    padDragging = true;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    handlePadPointer(e);
  }
  function onPadMove(e: PointerEvent) {
    if (!padDragging) return;
    handlePadPointer(e);
  }
  function onPadUp(e: PointerEvent) {
    padDragging = false;
    try {
      (e.target as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {}
  }

  // ---- Hex input: parse on blur or Enter.
  function onHexInput() {
    const val = hexInput.trim();
    if (!/^#?[0-9a-fA-F]{6}$/.test(val)) return;
    const normalised = val.startsWith("#") ? val : `#${val}`;
    const hsv = hexToHsv(normalised);
    if (hsv) commitHsv(hsv.h, hsv.s, hsv.v);
  }

  // ---- Conversions.
  function hsvToHex(hh: number, ss: number, vv: number): string {
    const c = vv * ss;
    const x = c * (1 - Math.abs(((hh / 60) % 2) - 1));
    const m = vv - c;
    let r = 0,
      g = 0,
      b = 0;
    if (hh < 60) [r, g, b] = [c, x, 0];
    else if (hh < 120) [r, g, b] = [x, c, 0];
    else if (hh < 180) [r, g, b] = [0, c, x];
    else if (hh < 240) [r, g, b] = [0, x, c];
    else if (hh < 300) [r, g, b] = [x, 0, c];
    else [r, g, b] = [c, 0, x];
    const toHex = (n: number) =>
      Math.round((n + m) * 255)
        .toString(16)
        .padStart(2, "0");
    return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
  }

  function hexToHsv(hex: string): { h: number; s: number; v: number } | null {
    const m = /^#?([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})$/.exec(hex);
    if (!m) return null;
    const r = parseInt(m[1] ?? "00", 16) / 255;
    const g = parseInt(m[2] ?? "00", 16) / 255;
    const b = parseInt(m[3] ?? "00", 16) / 255;
    const mx = Math.max(r, g, b);
    const mn = Math.min(r, g, b);
    const d = mx - mn;
    let hh = 0;
    if (d !== 0) {
      if (mx === r) hh = 60 * (((g - b) / d) % 6);
      else if (mx === g) hh = 60 * ((b - r) / d + 2);
      else hh = 60 * ((r - g) / d + 4);
    }
    if (hh < 0) hh += 360;
    const ss = mx === 0 ? 0 : d / mx;
    return { h: hh, s: ss, v: mx };
  }
</script>

<div class="cp-root">
  <!-- Saturation × value pad. Background is a solid hue tinted by
       white-vertical and black-horizontal gradients so a single
       coordinate fully encodes (s, v) at the current hue. -->
  <div
    bind:this={padEl}
    role="application"
    aria-label="Saturation and value pad: hue {Math.round(h)}, saturation {Math.round(s * 100)}%, value {Math.round(v * 100)}%"
    class="cp-pad"
    style="--pad-hue: {h}"
    onpointerdown={onPadDown}
    onpointermove={onPadMove}
    onpointerup={onPadUp}
    onpointercancel={onPadUp}
  >
    <div class="cp-pad-thumb" style="left: {s * 100}%; top: {(1 - v) * 100}%;"></div>
  </div>

  <!-- Hue slider. Gradient covers the full hue spectrum; thumb shows
       current hue position. Native `<input type=range>` styled
       transparent so the gradient track shows through. -->
  <div class="cp-hue-row">
    <input
      type="range"
      min="0"
      max="360"
      step="1"
      value={h}
      oninput={(e) => commitHsv(parseFloat(e.currentTarget.value), s, v)}
      class="cp-hue"
      aria-label="Hue"
    />
  </div>

  <!-- Hex text + preview swatch. The swatch doubles as the live
       output indicator; bind:value on the input keeps text and
       picker in sync per keystroke. -->
  <div class="cp-row">
    <div class="cp-swatch" style="background-color: {value};"></div>
    <input
      type="text"
      bind:value={hexInput}
      oninput={onHexInput}
      onkeydown={(e) => {
        if (e.key === "Enter") onHexInput();
      }}
      class="cp-hex"
      maxlength="7"
      spellcheck="false"
      aria-label="Hex colour"
    />
  </div>
</div>

<style>
  .cp-root {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    background: var(--color-bg-card, #171717);
    border: 1px solid var(--color-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 8px);
    box-shadow: var(--shadow-lg, 0 8px 32px rgba(0, 0, 0, 0.3));
    width: 220px;
  }

  .cp-pad {
    position: relative;
    height: 140px;
    border-radius: var(--radius-sm, 6px);
    background:
      linear-gradient(to bottom, transparent, #000),
      linear-gradient(to right, #fff, hsl(var(--pad-hue, 0), 100%, 50%));
    cursor: crosshair;
    touch-action: none;
  }

  .cp-pad-thumb {
    position: absolute;
    width: 12px;
    height: 12px;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    border: 2px solid #fff;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.4);
    pointer-events: none;
  }

  .cp-hue-row {
    display: flex;
  }

  .cp-hue {
    flex: 1;
    appearance: none;
    height: 12px;
    border-radius: 999px;
    background: linear-gradient(
      to right,
      hsl(0, 100%, 50%),
      hsl(60, 100%, 50%),
      hsl(120, 100%, 50%),
      hsl(180, 100%, 50%),
      hsl(240, 100%, 50%),
      hsl(300, 100%, 50%),
      hsl(360, 100%, 50%)
    );
    margin: 0;
    cursor: pointer;
  }

  .cp-hue::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    border: 2px solid rgba(0, 0, 0, 0.5);
    cursor: pointer;
  }
  .cp-hue::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    border: 2px solid rgba(0, 0, 0, 0.5);
    cursor: pointer;
  }

  .cp-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .cp-swatch {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm, 6px);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell, #fff) 15%, transparent);
    flex-shrink: 0;
  }

  .cp-hex {
    flex: 1;
    height: 28px;
    background: color-mix(in srgb, var(--color-fg-shell, #fff) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell, #fff) 10%, transparent);
    border-radius: var(--radius-sm, 6px);
    color: var(--color-fg-shell, #fff);
    font-family: ui-monospace, SFMono-Regular, monospace;
    font-size: 0.85rem;
    padding: 0 8px;
    outline: none;
  }

  .cp-hex:focus {
    border-color: var(--color-accent, #6366f1);
  }
</style>
