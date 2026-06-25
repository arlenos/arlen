/// Block-chrome builders: the engine registers the xterm decorations and hands
/// each `onRender` element here so the Arlen look (accent, result, run-again)
/// fills it. The styles live in `block-chrome.css`. xterm draws the real shell
/// prompt, so the chrome never re-renders cwd/git - it adds only the boundary,
/// the result, and the run-again affordance.

/// The block boundary bar. Anchor the decoration at the prompt-start row, x=0,
/// width 1, height = the block's row count, then pass its element here.
export function applyBlockAccent(
  el: HTMLElement,
  opts: { isError?: boolean; isActive?: boolean } = {},
): void {
  el.classList.add("arlen-block-accent");
  el.classList.toggle("is-error", !!opts.isError);
  el.classList.toggle("is-active", !!opts.isActive);
}

/// The result strip: the duration always once finished, an exit chip only on a
/// non-zero exit (the quiet-chrome rule - the absence of an error is the
/// status), and a run-again button that surfaces on block hover. Anchor the
/// decoration to the prompt row, `anchor: "right"`, then pass its element here.
/// `onRerun` runs the command again (the engine wires the actual PTY write).
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
