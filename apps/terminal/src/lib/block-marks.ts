/// Pure parsing of the shell's block-boundary OSC marks, shared by the terminal
/// grid's decoration handler (Terminal.svelte) and its tests.
///
/// Both the FinalTerm OSC 133 and the VS Code OSC 633 families carry the same
/// A/C/D block marks. The Arlen shell integration (arlen-shell-integration.zsh)
/// emits `633;A` for prompt-start and `133;C` / `133;D;<exit>` for exec/end (and
/// `633;E;<cmd>` for the command line, which the grid ignores - the engine
/// decodes + nonce-validates that for the trusted block record). So the grid
/// registers ONE classifier on both OSC 133 and 633 and routes by the leading
/// letter, family-agnostic. Keeping that here as a pure function lets the
/// regression be tested without an xterm instance: the earlier bug was the
/// handler firing only on `133;A`, so a 633-only prompt-start never opened a
/// block and nothing rendered.

/// A recognised block boundary; anything else (prompt-end `B`, the command-line
/// `E`, property marks, …) is not a boundary the grid chrome acts on.
export type BlockMark = "prompt-start" | "exec-start" | "command-end";

/// Classify an OSC 133/633 data payload (everything after the `133;`/`633;`)
/// into a block boundary, or null when it is not one the chrome cares about.
/// Accepts both the bare letter (`A`) and the parameterised form (`A;…`).
export function classifyMark(data: string): BlockMark | null {
  if (data === "A" || data.startsWith("A;")) return "prompt-start";
  if (data === "C" || data.startsWith("C;")) return "exec-start";
  if (data === "D" || data.startsWith("D;")) return "command-end";
  return null;
}

/// The integer exit code carried by a `D[;<exit>]` command-end payload, or null
/// when absent (a bare `D`) or malformed (a non-integer field). Used to tint the
/// finished block's accent bar and label the result chip.
export function parseExitCode(data: string): number | null {
  const semi = data.indexOf(";");
  if (semi < 0) return null;
  const code = Number.parseInt(data.slice(semi + 1), 10);
  return Number.isInteger(code) ? code : null;
}
