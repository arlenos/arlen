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

export async function terminalGrid(sessionId: string): Promise<GridSnapshot> {
  return invoke<GridSnapshot>("terminal_grid", { sessionId });
}

export async function terminalInput(
  sessionId: string,
  input: string,
): Promise<void> {
  await invoke("terminal_input", { sessionId, input });
}

export async function terminalNewSession(): Promise<Session> {
  return invoke<Session>("terminal_new_session");
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

/// Reads the AI capability context; null when the backend is
/// unreachable (the composer strip renders that state distinctly).
export async function readCapability(): Promise<Capability | null> {
  try {
    return await invoke<Capability>("ai_capability");
  } catch {
    return null;
  }
}
