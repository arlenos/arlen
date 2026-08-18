<script lang="ts">
  /// The per-logical-core grid (system-monitor-plan.md (b)), on canvas for the
  /// same reason `Graph.svelte` is: a reactive DOM node per core would make the
  /// webview the top process in its own list, on the very screen you opened to
  /// find out what is.
  ///
  /// Each cell is one core, drawn as a stacked bar rather than a single height,
  /// because the three shares mean different things. User is the program doing
  /// work. System is the kernel doing work on its behalf - a core pinned there
  /// is a different problem from one pinned in user. Iowait is not work at all:
  /// the core is idle and something on it is blocked on a disk, so it is drawn
  /// in a distinctly dimmer shade and never adds to a "busy" reading.
  import type { CoreUsage } from "$lib/stores/perf";
  import { columnsFor } from "$lib/core-grid";

  let { cores }: { cores: CoreUsage[] } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();

  $effect(() => {
    draw(cores);
  });

  function draw(list: CoreUsage[]): void {
    if (!canvas) return;
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
    if (list.length === 0) return;

    const fg = getComputedStyle(canvas).getPropertyValue("--color-fg-primary").trim() || "#fafafa";
    const cols = columnsFor(list.length, w, h);
    const rows = Math.ceil(list.length / cols);
    const gap = 4;
    const cw = (w - gap * (cols - 1)) / cols;
    const ch = (h - gap * (rows - 1)) / rows;

    list.forEach((c, i) => {
      const x = (i % cols) * (cw + gap);
      const y = Math.floor(i / cols) * (ch + gap);

      // The cell's own ground, so an idle core is a visible box rather than a
      // gap - the grid should show how many cores there are, not only the busy
      // ones.
      ctx.globalAlpha = 0.08;
      ctx.fillStyle = fg;
      ctx.fillRect(x, y, cw, ch);

      // Stacked from the bottom: user, then system on top of it, then iowait.
      // Clamped as a stack rather than per band, so rounding cannot draw past
      // the cell.
      let used = 0;
      const band = (share: number, alpha: number) => {
        const pct = Math.max(0, Math.min(share, 100 - used));
        if (pct <= 0) return;
        const bh = (pct / 100) * ch;
        ctx.globalAlpha = alpha;
        ctx.fillRect(x, y + ch - (used / 100) * ch - bh, cw, bh);
        used += pct;
      };
      band(c.user, 0.95);
      band(c.system, 0.55);
      band(c.iowait, 0.22);
    });
    ctx.globalAlpha = 1;
  }
</script>

<canvas bind:this={canvas} class="grid" aria-hidden="true"></canvas>

<style>
  .grid {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
