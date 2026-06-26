/// The terminal IPC contract, mirroring `apps/terminal/core/src/lib.rs`
/// field for field (all enums serialize lowercase via serde). The UI is
/// built against these shapes; the engine fills them in later. Do not
/// extend here — contract changes happen in the core crate first.

import { invoke } from "@tauri-apps/api/core";

/// Who issued the command in a block.
export type Origin = "you" | "agent";

/// What a block's body is: plain terminal text painted by the grid
/// (a reserved transparent hole), or a GUI component.
export type BlockBodyKind =
  | "grid"
  | "table"
  | "image"
  | "link"
  | "artifact"
  | "widget";

/// The lifecycle of a shell session.
export type SessionStatus = "running" | "exited";

/// The git state of a block's working directory.
export interface GitInfo {
  branch: string;
  dirty_count: number;
}

/// One command plus its result: the unit the terminal renders as a
/// block. `exit_code` and `duration_ms` are null while the command is
/// still running. `body` is opaque to the contract; the UI dispatches
/// on `body_kind` only.
export interface Block {
  id: string;
  command: string;
  exit_code: number | null;
  duration_ms: number | null;
  cwd: string;
  git: GitInfo | null;
  origin: Origin;
  body_kind: BlockBodyKind;
  body: unknown;
  /// The shell's own rendered prompt line (the cells the shell actually
  /// printed for the prompt plus the echoed command, with their colours and
  /// syntax highlighting), captured between the prompt-start and exec-start
  /// marks. The block header renders these verbatim, so the record is exactly
  /// what was on screen - any prompt (p10k, starship, custom), not a themed
  /// reconstruction. Null for an older block or before the engine captures it;
  /// the header then falls back to the regenerated path+git line.
  prompt_cells?: GridCell[][] | null;
}

/// A terminal cell's colour (mirrors core `CellColor`): the theme default, a
/// 256-palette index, or a direct RGB triple. The webview maps it to CSS.
export type CellColor =
  | { kind: "default" }
  | { kind: "indexed"; value: number }
  | { kind: "rgb"; value: [number, number, number] };

/// One visible terminal cell (mirrors core `GridCell`): a glyph plus its SGR
/// styling. `text` is empty for a blank cell; `wide` marks the lead half of a
/// double-width glyph.
export interface GridCell {
  text: string;
  fg: CellColor;
  bg: CellColor;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  inverse: boolean;
  wide: boolean;
}

/// A point-in-time view of the terminal screen (the Rust `GridSnapshot`): the
/// visible grid as rows of styled cells plus the geometry and cursor. The
/// webview paints these cells (with colour and alignment) so command output
/// appears without the compositor grid-subsurface (terminal.md Option B).
export interface GridSnapshot {
  cols: number;
  rows: number;
  cells: GridCell[][];
  /// Whether a fullscreen / TUI app holds the alternate screen; the renderer
  /// paints the full grid (no trailing-row trimming) when this is set.
  alt_screen: boolean;
  cursor_row: number;
  cursor_col: number;
  /// Whether the cursor should be drawn (the VT SHOW_CURSOR mode). The live
  /// region paints a block cursor at (cursor_row, cursor_col) only when set, so
  /// a TUI that hides its cursor (btop) gets no spurious block over its frame.
  cursor_visible: boolean;
  /// Whether a command is running (its OSC 133;C mark seen, 133;D not yet).
  /// Lets the renderer tell an in-flight command's output from an idle prompt,
  /// so the shell's prompt is never drawn under the block-model composer.
  running: boolean;
  /// The grid row where the running command's output begins (cursor row at the
  /// ExecStart mark, past the prompt + command echo); null at an idle prompt.
  output_start_row: number | null;
  /// The grid row where the current prompt begins (cursor row at the PromptStart
  /// 133;A mark), cleared at ExecStart; null while a command runs or before the
  /// first marked prompt. The live region renders from here at an idle prompt so
  /// the shell's prompt + the line being typed are the interactive surface.
  prompt_start_row: number | null;
}

/// A running (or finished) shell, surfaced as a tab in the sidebar.
export interface Session {
  id: string;
  cwd: string;
  status: SessionStatus;
  last_exit: number | null;
}

/// A project the terminal can scope history and sessions to.
export interface Project {
  id: string;
  name: string;
  path: string;
}

/// Filters for a history search over past blocks. All fields default
/// off, so an empty set matches every block.
export interface HistoryFilters {
  cwd: string | null;
  origin: Origin | null;
  project_id: string | null;
  only_failures: boolean;
}

export function emptyFilters(): HistoryFilters {
  return { cwd: null, origin: null, project_id: null, only_failures: false };
}

/// The AI capability context (same command and shape as the harness;
/// serde renames to camelCase on the wire).
export interface Capability {
  enabled: boolean;
  tier: string;
  actionMode: string;
  provider?: string | null;
  model?: string | null;
  executorLive: boolean;
}

// ── Command wrappers ────────────────────────────────────────────────

export async function terminalSessions(): Promise<Session[]> {
  return invoke<Session[]>("terminal_sessions");
}

export async function terminalBlocks(sessionId: string): Promise<Block[]> {
  return invoke<Block[]>("terminal_blocks", { sessionId });
}

/// The latest assembled block's command, for run-again - a light read that
/// avoids re-serialising every block the way `terminalBlocks` does. null when
/// there is no session or no finished command yet.
export async function terminalLastCommand(sessionId: string): Promise<string | null> {
  return invoke<string | null>("terminal_last_command", { sessionId });
}

export async function terminalGrid(sessionId: string): Promise<GridSnapshot> {
  return invoke<GridSnapshot>("terminal_grid", { sessionId });
}

/// Drain the session's raw PTY output bytes for the xterm.js renderer: the bytes
/// read since the last call (a JSON number array over the wire). The caller
/// writes them to its xterm.js instance, which owns the VT parsing + render. The
/// frontend pulls this on each `terminal://frame` signal.
export async function terminalDrainOutput(sessionId: string): Promise<number[]> {
  return invoke<number[]>("terminal_drain_output", { sessionId });
}

export async function terminalInput(
  sessionId: string,
  input: string,
): Promise<void> {
  await invoke("terminal_input", { sessionId, input });
}

/// Resize the session's PTY to `cols`x`rows` (the engine resizes the master PTY,
/// sending SIGWINCH so the shell + TUIs reflow, and the VT parser to match). The
/// page computes the grid size from the rendered cell metrics on a resize.
export async function terminalResize(
  sessionId: string,
  cols: number,
  rows: number,
): Promise<void> {
  await invoke("terminal_resize", { sessionId, cols, rows });
}

export async function terminalNewSession(): Promise<Session> {
  return invoke<Session>("terminal_new_session");
}

/// Save a block's output to a timestamped file (under ~/Downloads, else $HOME);
/// returns the saved path. A save-as dialog is the later enhancement.
export async function terminalSaveOutput(content: string): Promise<string> {
  return invoke<string>("terminal_save_output", { content });
}

/// Close a session in the backend (reaps the dead shell from the registry).
export async function terminalCloseSession(sessionId: string): Promise<void> {
  await invoke("terminal_close_session", { sessionId });
}

export async function terminalHistorySearch(
  query: string,
  filters: HistoryFilters,
): Promise<Block[]> {
  return invoke<Block[]>("terminal_history_search", { query, filters });
}

export async function terminalProjects(): Promise<Project[]> {
  return invoke<Project[]>("terminal_projects");
}

/// The persisted terminal config (terminal-ui-plan.md §5b). `font_size` is the
/// base monospace size in px the grid renders at; the daemon clamps it.
export interface TerminalConfig {
  font_size: number;
}

/// Read the persisted base font size (the daemon falls back to its default when
/// the config is absent or invalid).
export async function terminalConfigGet(): Promise<TerminalConfig> {
  return invoke<TerminalConfig>("terminal_config_get");
}

/// Persist the base font size (the Settings UI; the daemon clamps it to a
/// readable range). Zoom shortcuts apply a transient delta over this base and do
/// not call this.
export async function terminalConfigSet(fontSize: number): Promise<void> {
  await invoke("terminal_config_set", { fontSize });
}

/// Reads the AI capability context; null when the backend is
/// unreachable (the composer strip renders that state distinctly).
export async function readCapability(): Promise<Capability | null> {
  try {
    return await invoke<Capability>("ai_capability");
  } catch {
    return null;
  }
}
