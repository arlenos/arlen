/// Block-chrome builders: the engine registers the xterm decorations and hands
/// each `onRender` element here so the Arlen look (hover tint, result chip) fills
/// it. The styles live in `block-chrome.css`. xterm draws the real shell prompt,
/// so the chrome never re-renders cwd/git - it adds only the hover boundary and
/// the failed-exit chip; block actions live in the right-click block menu.

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

/// The result strip: an exit chip ONLY on a non-zero exit, nothing on success
/// (the quiet-chrome rule - the absence of an error is the status). The duration
/// and run-again are deliberately NOT here: the shell prompt already shows the
/// command duration (the Arlen starship right-prompt carries `cmd_duration`), and
/// the block actions (run-again, copy command / response / history) move to a
/// right-click menu on the hovered block. `durationMs` / `onRerun` are accepted
/// but unused while that wiring is migrated off the inline button.
export function renderBlockResult(
  el: HTMLElement,
  opts: {
    exitCode: number | null;
    durationMs?: number | null;
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
}
