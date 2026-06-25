/// Block-chrome builders: the engine registers the xterm decorations and hands
/// each `onRender` element here so the Arlen look (hover tint, result, run-again)
/// fills it. The styles live in `block-chrome.css`. xterm draws the real shell
/// prompt, so the chrome never re-renders cwd/git - it adds only the hover
/// boundary, the result, and the run-again affordance.

/// The hover tint. While the pointer is over a block, the engine registers a
/// decoration spanning the block's rows (prompt row through the last output row)
/// and passes its element here; `isError` warms the wash toward the error colour.
/// At rest no such decoration exists - the terminal is pure.
export function applyBlockHover(
  el: HTMLElement,
  opts: { isError?: boolean } = {},
): void {
  el.classList.add("arlen-block-hover");
  el.classList.toggle("is-error", !!opts.isError);
}

/// The result strip: the duration always once finished, an exit chip only on a
/// non-zero exit (the quiet-chrome rule - the absence of an error is the
/// status), and a run-again button that surfaces only while the block is hovered.
/// Anchor the decoration to the prompt row full-width (`x: 0, width: cols`), then
/// pass its element here. `onRerun` runs the command again (the engine wires the
/// actual PTY write). Toggle `is-hover` on this element in step with the block
/// hover to reveal run-again.
export function renderBlockResult(
  el: HTMLElement,
  opts: {
    exitCode: number | null;
    durationMs: number | null;
    onRerun?: () => void;
  },
): void {
  el.classList.add("arlen-block-result");
  el.replaceChildren();

  if (opts.exitCode !== null && opts.exitCode !== 0) {
    const exit = document.createElement("span");
    exit.className = "arlen-block-exit";
    exit.textContent = `exit ${opts.exitCode}`;
    el.append(exit);
  }

  if (opts.durationMs !== null) {
    const dur = document.createElement("span");
    dur.className = "arlen-block-duration";
    dur.textContent = formatDuration(opts.durationMs);
    el.append(dur);
  }

  if (opts.onRerun) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "arlen-block-rerun";
    btn.setAttribute("aria-label", "Run this command again");
    // A plain rotate glyph - no icon dependency in this imperative DOM.
    btn.textContent = "↻";
    btn.addEventListener("click", opts.onRerun);
    el.append(btn);
  }
}

/// Wall-clock duration for the result chip: sub-second in ms, then a trimmed
/// seconds value, then minutes - the terminal convention.
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  const m = Math.floor(s / 60);
  const rem = Math.round(s % 60);
  return `${m}m ${rem}s`;
}
