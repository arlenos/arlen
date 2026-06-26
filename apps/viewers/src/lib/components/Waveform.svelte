<script lang="ts">
  /// The audio file's face: one custom canvas that is visualizer, progress and
  /// scrubber at once (quickview-plan.md). The played span is bright
  /// (`--color-fg-primary`), the rest low-alpha (the terminal-hover idiom); a
  /// click seeks; a subtle shimmer rides the playhead so it feels alive. Drawn
  /// as one smooth filled silhouette on a canvas, never one DOM node per sample.
  import { onMount } from "svelte";

  let {
    peaks,
    progress = 0,
    onseek,
  }: {
    /// Normalised 0..1 amplitude, one entry per bar.
    peaks: number[];
    /// Playback position 0..1 (the bright/dim split).
    progress?: number;
    /// Seek to a 0..1 fraction of the track (click on the wave).
    onseek?: (fraction: number) => void;
  } = $props();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let raf = 0;

  /// Read a CSS custom property as an `[r,g,b]` triple (so the wave follows the
  /// theme's foreground rather than a hardcoded white).
  function fgRgb(): [number, number, number] {
    const v = getComputedStyle(host).getPropertyValue("--color-fg-primary").trim() || "#fafafa";
    const h = v.replace("#", "");
    const n = h.length === 3 ? h.split("").map((c) => c + c).join("") : h;
    const int = parseInt(n.slice(0, 6), 16);
    return [(int >> 16) & 255, (int >> 8) & 255, int & 255];
  }

  function draw(now: number) {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const w = host.clientWidth;
    const h = host.clientHeight;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const n = peaks.length;
    if (n === 0) return;
    const mid = h / 2;
    const amp = mid * 0.94;
    const step = w / (n - 1);
    const [r, g, b] = fgRgb();
    const playX = progress * w;

    // One smooth closed silhouette: the top edge left-to-right through the
    // peak midpoints (quadratic smoothing), then the mirrored bottom edge back,
    // closed and filled - a single continuous shape, not discrete bars.
    const trace = () => {
      ctx.beginPath();
      ctx.moveTo(0, mid - peaks[0] * amp);
      for (let i = 1; i < n; i++) {
        const x0 = (i - 1) * step;
        const y0 = mid - peaks[i - 1] * amp;
        const xc = (x0 + i * step) / 2;
        const yc = (y0 + (mid - peaks[i] * amp)) / 2;
        ctx.quadraticCurveTo(x0, y0, xc, yc);
      }
      ctx.lineTo(w, mid - peaks[n - 1] * amp);
      ctx.lineTo(w, mid + peaks[n - 1] * amp);
      for (let i = n - 1; i > 0; i--) {
        const x0 = i * step;
        const y0 = mid + peaks[i] * amp;
        const xc = (x0 + (i - 1) * step) / 2;
        const yc = (y0 + (mid + peaks[i - 1] * amp)) / 2;
        ctx.quadraticCurveTo(x0, y0, xc, yc);
      }
      ctx.lineTo(0, mid + peaks[0] * amp);
      ctx.closePath();
    };

    // Dim whole, then the played span bright (clip at the playhead).
    ctx.save();
    trace();
    ctx.fillStyle = `rgba(${r},${g},${b},0.2)`;
    ctx.fill();
    ctx.restore();

    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, playX, h);
    ctx.clip();
    trace();
    ctx.fillStyle = `rgba(${r},${g},${b},0.95)`;
    ctx.fill();
    ctx.restore();

    // A soft highlight breathing just behind the playhead, so it feels live.
    const gw = 44;
    const x0 = Math.max(0, playX - gw);
    if (playX > 0) {
      const pulse = 0.5 + 0.5 * Math.sin(now / 460);
      const grad = ctx.createLinearGradient(playX - gw, 0, playX, 0);
      grad.addColorStop(0, `rgba(${r},${g},${b},0)`);
      grad.addColorStop(1, `rgba(${r},${g},${b},${0.28 * pulse})`);
      ctx.save();
      ctx.beginPath();
      ctx.rect(x0, 0, playX - x0, h);
      ctx.clip();
      trace();
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.restore();
    }

    raf = requestAnimationFrame(draw);
  }

  function seek(e: MouseEvent) {
    const rect = host.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    onseek?.(f);
  }

  onMount(() => {
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  });
</script>

<div
  bind:this={host}
  class="wave"
  role="slider"
  tabindex="0"
  aria-label="Seek"
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={Math.round(progress * 100)}
  onclick={seek}
  onkeydown={(e) => {
    if (e.key === "ArrowRight") onseek?.(Math.min(1, progress + 0.02));
    else if (e.key === "ArrowLeft") onseek?.(Math.max(0, progress - 0.02));
  }}
>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .wave {
    position: relative;
    width: 100%;
    height: 100%;
    cursor: pointer;
    outline: none;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
