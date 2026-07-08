<script lang="ts">
  /// A live device graph on canvas (never a reactive SVG - that would make the
  /// webview the top process). Draws a number[] series as a monochrome filled area
  /// line, redrawn only when the series ticks. Two sizes: `big` (the main pane) and
  /// `spark` (the device-list mini graph).
  let {
    series,
    max,
    variant = "big",
  }: { series: number[]; max: number; variant?: "big" | "spark" } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();

  $effect(() => {
    // Reading `series` here makes the effect re-run on every tick.
    draw(series);
  });

  function draw(s: number[]): void {
    if (!canvas || s.length < 2) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    if (w === 0 || h === 0) return;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const fg = getComputedStyle(canvas).getPropertyValue("--color-fg-primary").trim() || "#fafafa";
    const n = s.length;
    const x = (i: number) => (i / (n - 1)) * w;
    const y = (v: number) => h - (Math.max(0, Math.min(v, max)) / max) * h;

    if (variant === "big") {
      // Faint horizontal gridlines.
      ctx.strokeStyle = fg;
      ctx.globalAlpha = 0.06;
      ctx.lineWidth = 1;
      for (let g = 1; g < 4; g++) {
        const gy = (g / 4) * h;
        ctx.beginPath();
        ctx.moveTo(0, gy);
        ctx.lineTo(w, gy);
        ctx.stroke();
      }
    }

    // The area fill.
    ctx.beginPath();
    ctx.moveTo(0, h);
    for (let i = 0; i < n; i++) ctx.lineTo(x(i), y(s[i]));
    ctx.lineTo(w, h);
    ctx.closePath();
    ctx.fillStyle = fg;
    ctx.globalAlpha = variant === "big" ? 0.12 : 0.1;
    ctx.fill();

    // The line.
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const px = x(i);
      const py = y(s[i]);
      i ? ctx.lineTo(px, py) : ctx.moveTo(px, py);
    }
    ctx.globalAlpha = variant === "big" ? 0.8 : 0.6;
    ctx.strokeStyle = fg;
    ctx.lineWidth = variant === "big" ? 1.5 : 1;
    ctx.lineJoin = "round";
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
</script>

<canvas bind:this={canvas} class="graph {variant}"></canvas>

<style>
  .graph {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
