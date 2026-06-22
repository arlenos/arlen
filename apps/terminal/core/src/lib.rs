//! The Arlen terminal IPC contract: the data shapes the terminal backend and its
//! UI agree on (`terminal-ui-plan.md` §5).
//!
//! These types are the **embeddable core**. Both `apps/terminal` and an embedded
//! terminal pane (the file manager hosts one, FM-R6, with bidirectional cwd sync)
//! speak them, so they live in this Tauri-agnostic crate rather than the app
//! shell, and are never woven into one host and retrofitted. The arlen-ui session
//! mirrors these shapes in its mock until the real backend (TM-R1) wires them.
//!
//! # The command surface
//!
//! The Tauri host (`apps/terminal/src-tauri`, the thin host of TM-R1) exposes
//! these commands over the types in this crate; the contract the UI mocks is:
//!
//! - `terminal_sessions() -> Vec<Session>` — the open shells (sidebar tabs).
//! - `terminal_blocks(session_id: String) -> Vec<Block>` — a session's blocks.
//! - `terminal_input(session_id: String, input: String) -> Result<(), String>` —
//!   feed input to a session's shell.
//! - `terminal_new_session() -> Result<Session, String>` — open a new shell.
//! - `terminal_history_search(query: String, filters: HistoryFilters) -> Vec<Block>`
//!   — the `⌃R` search over past blocks.
//! - `terminal_projects() -> Vec<Project>` — the graph-backed projects to scope to.
//! - `ai_capability()` — the `◆` capability indicator, reused from the harness.

use serde::{Deserialize, Serialize};

pub mod blocks;
pub mod vt;

/// Who issued the command in a block: the user typed it, or the agent ran it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// The user typed the command.
    You,
    /// The AI agent ran the command on the user's behalf.
    Agent,
}

/// What a block's body is, so the UI knows whether to reserve a transparent grid
/// hole (text) or render a GUI component (everything else). `terminal-ui-plan.md`
/// §3-§5: the backend ships one bit per block, text-grid vs GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockBodyKind {
    /// Plain terminal text, painted by the fast grid (a reserved transparent hole
    /// in the DOM). The default for ordinary command output.
    Grid,
    /// A structured table the UI renders as a real grid component.
    Table,
    /// An image (Kitty/iTerm graphics, or an explain/artifact image).
    Image,
    /// A link card.
    Link,
    /// An artifact (the arlen-artifact system).
    Artifact,
    /// An agent-drafted interactive widget.
    Widget,
}

/// A terminal cell's colour: the theme default, a 256-palette index, or a direct
/// RGB triple. Serialized adjacently-tagged (`kind` + `value`) so the webview maps
/// it to CSS: the ANSI/256 palette for `indexed`, `rgb(...)` for `rgb`, and the
/// theme's own foreground/background for `default`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "value")]
pub enum CellColor {
    /// Use the theme default (the SGR reset colour).
    #[default]
    Default,
    /// A 256-colour palette index (0-15 ANSI, 16-255 the xterm cube + greys).
    Indexed(u8),
    /// A direct 24-bit RGB colour.
    Rgb([u8; 3]),
}

/// One visible terminal cell: its glyph plus the SGR styling the webview paints.
/// `text` is empty for a blank cell. `wide` marks the lead half of a
/// double-width glyph (its following continuation cell carries empty `text`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridCell {
    /// The cell's glyph(s); empty for a blank cell.
    pub text: String,
    /// Foreground colour.
    pub fg: CellColor,
    /// Background colour.
    pub bg: CellColor,
    /// SGR bold.
    pub bold: bool,
    /// SGR italic.
    pub italic: bool,
    /// SGR underline.
    pub underline: bool,
    /// SGR inverse: the renderer swaps fg and bg.
    pub inverse: bool,
    /// Lead half of a double-width glyph (render two columns wide).
    pub wide: bool,
}

/// A point-in-time view of the terminal screen: the visible grid as rows of
/// styled cells plus the geometry and cursor. The webview paints these cells so
/// command output appears (with colour and alignment) without the compositor
/// grid-subsurface (terminal.md Option B, the portable path); the subsurface
/// stays the later performance optimization.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridSnapshot {
    /// Visible columns.
    pub cols: u16,
    /// Visible rows.
    pub rows: u16,
    /// The visible grid, top to bottom: one inner vector per row, each holding
    /// exactly `cols` cells left to right (so the monospace grid always aligns).
    pub cells: Vec<Vec<GridCell>>,
    /// Whether the alternate screen is active (a fullscreen / TUI app like vim
    /// or less has taken over the screen). The renderer paints the full grid
    /// without trimming trailing blank rows when this is set, since the app
    /// owns the whole screen rather than appending command output.
    pub alt_screen: bool,
    /// Cursor row (0-based, from the top of the visible screen).
    pub cursor_row: u16,
    /// Cursor column (0-based).
    pub cursor_col: u16,
}

/// The git state of a block's working directory, when it is a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitInfo {
    /// The current branch name.
    pub branch: String,
    /// The number of dirty (modified or untracked) entries.
    pub dirty_count: u32,
}

/// One command plus its result: the unit the terminal renders as a block. A block
/// is the projection of a future KG command node carrying its cwd, exit, timing
/// and origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Stable id of the block.
    pub id: String,
    /// The command line that ran.
    pub command: String,
    /// The exit code, once the command finishes; `None` while it is still
    /// running.
    pub exit_code: Option<i32>,
    /// Wall-clock duration in milliseconds, once finished; `None` while running.
    pub duration_ms: Option<u64>,
    /// The working directory the command ran in.
    pub cwd: String,
    /// The git state of `cwd`, or `None` when it is not a repository.
    pub git: Option<GitInfo>,
    /// Who issued the command.
    pub origin: Origin,
    /// What kind of body this block carries (text grid vs a GUI component).
    pub body_kind: BlockBodyKind,
    /// The body payload, interpreted per `body_kind` and opaque to this contract:
    /// grid text for [`BlockBodyKind::Grid`], the component model for the GUI
    /// kinds. The UI dispatches on `body_kind`, so the wire shape stays one field.
    pub body: serde_json::Value,
}

/// The lifecycle of a shell session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// The shell is alive.
    Running,
    /// The shell has exited.
    Exited,
}

/// A running (or finished) shell, surfaced as a tab in the sidebar with its cwd,
/// status and last exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Stable id of the session.
    pub id: String,
    /// The session's current working directory.
    pub cwd: String,
    /// Whether the shell is alive or has exited.
    pub status: SessionStatus,
    /// The exit code of the last finished command, or `None` if none has run.
    pub last_exit: Option<i32>,
}

/// A project the terminal can scope history and sessions to (graph-backed; the
/// projection of a KG `Project`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Stable id of the project.
    pub id: String,
    /// Human-readable project name.
    pub name: String,
    /// The project's root path.
    pub path: String,
}

/// Filters for a history (`⌃R`) search over past blocks. All fields are
/// optional/default-off, so an empty filter set matches every block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryFilters {
    /// Restrict to blocks whose working directory is at or under this path.
    pub cwd: Option<String>,
    /// Restrict to a given origin (the user or the agent).
    pub origin: Option<Origin>,
    /// Restrict to a given project id.
    pub project_id: Option<String>,
    /// Only blocks that failed (a non-zero exit code).
    pub only_failures: bool,
}

/// The contract command handlers, as stubs.
///
/// These are the backend behind the `terminal_*` Tauri commands. They live in
/// the embeddable core (not the app shell) because TM-R1's `VtEngine` replaces
/// each stub with the real, engine-backed implementation, and the file manager's
/// embedded terminal pane (FM-R6) drives the same handlers. The Tauri host
/// (`apps/terminal/src-tauri`, the thin host) wraps each as a one-line
/// `#[tauri::command]` of the matching name. Until the engine wires, the queries
/// report nothing (the UI's empty state) rather than fabricate data; arlen-ui
/// renders the populated chrome against its own mock of these shapes.
pub mod stub {
    use super::{Block, HistoryFilters, Project, Session, SessionStatus};

    /// Stub for `terminal_sessions()`: the open shells. Empty until the engine
    /// spawns and tracks them.
    pub fn sessions() -> Vec<Session> {
        Vec::new()
    }

    /// Stub for `terminal_blocks(session_id)`: a session's blocks. Empty until the
    /// engine surfaces OSC-marked command output.
    pub fn blocks(_session_id: &str) -> Vec<Block> {
        Vec::new()
    }

    /// Stub for `terminal_input(session_id, input)`: feed input to a session's
    /// shell. A no-op until the engine owns the pty.
    pub fn input(_session_id: &str, _input: &str) -> Result<(), String> {
        Ok(())
    }

    /// Stub for `terminal_new_session()`: open a new shell. Returns a fresh
    /// running session anchored at the user's home so the shape is exercised
    /// (the engine assigns the real id and cwd).
    pub fn new_session() -> Result<Session, String> {
        Ok(Session {
            id: String::new(),
            cwd: std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
            status: SessionStatus::Running,
            last_exit: None,
        })
    }

    /// Stub for `terminal_history_search(query, filters)`: the `⌃R` search over
    /// past blocks. Empty until the graph-backed history is wired.
    pub fn history_search(_query: &str, _filters: &HistoryFilters) -> Vec<Block> {
        Vec::new()
    }

    /// Stub for `terminal_projects()`: the projects to scope to. Empty until the
    /// graph-backed project list is wired.
    pub fn projects() -> Vec<Project> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn block_serializes_to_the_exact_contract_shape() {
        let b = Block {
            id: "b1".into(),
            command: "ls -la".into(),
            exit_code: Some(0),
            duration_ms: Some(12),
            cwd: "/home/x".into(),
            git: Some(GitInfo {
                branch: "main".into(),
                dirty_count: 3,
            }),
            origin: Origin::You,
            body_kind: BlockBodyKind::Grid,
            body: json!({ "text": "total 0" }),
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(
            v,
            json!({
                "id": "b1",
                "command": "ls -la",
                "exit_code": 0,
                "duration_ms": 12,
                "cwd": "/home/x",
                "git": { "branch": "main", "dirty_count": 3 },
                "origin": "you",
                "body_kind": "grid",
                "body": { "text": "total 0" }
            })
        );
        // The shape round-trips, so the UI's mock and the real backend agree.
        assert_eq!(serde_json::from_value::<Block>(v).unwrap(), b);
    }

    #[test]
    fn a_running_block_reports_null_exit_duration_and_git() {
        let b = Block {
            id: "b".into(),
            command: "sleep 9".into(),
            exit_code: None,
            duration_ms: None,
            cwd: "/".into(),
            git: None,
            origin: Origin::Agent,
            body_kind: BlockBodyKind::Grid,
            body: json!(null),
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["exit_code"], json!(null));
        assert_eq!(v["duration_ms"], json!(null));
        assert_eq!(v["git"], json!(null));
        assert_eq!(v["origin"], json!("agent"));
    }

    #[test]
    fn body_kinds_render_to_the_contract_strings() {
        for (kind, s) in [
            (BlockBodyKind::Grid, "grid"),
            (BlockBodyKind::Table, "table"),
            (BlockBodyKind::Image, "image"),
            (BlockBodyKind::Link, "link"),
            (BlockBodyKind::Artifact, "artifact"),
            (BlockBodyKind::Widget, "widget"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(s));
        }
    }

    #[test]
    fn session_serializes_to_the_exact_contract_shape() {
        let s = Session {
            id: "s1".into(),
            cwd: "/w".into(),
            status: SessionStatus::Running,
            last_exit: Some(1),
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({ "id": "s1", "cwd": "/w", "status": "running", "last_exit": 1 })
        );
        assert_eq!(
            serde_json::to_value(SessionStatus::Exited).unwrap(),
            json!("exited")
        );
    }

    #[test]
    fn an_empty_history_filter_is_the_default_and_round_trips() {
        let f = HistoryFilters::default();
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(
            v,
            json!({ "cwd": null, "origin": null, "project_id": null, "only_failures": false })
        );
        assert_eq!(serde_json::from_value::<HistoryFilters>(v).unwrap(), f);
    }

    #[test]
    fn stub_handlers_match_the_command_signatures() {
        // The query stubs report nothing until the engine wires (no fabricated
        // data), and the mutating stubs succeed.
        assert!(stub::sessions().is_empty());
        assert!(stub::blocks("s1").is_empty());
        assert!(stub::history_search("git", &HistoryFilters::default()).is_empty());
        assert!(stub::projects().is_empty());
        assert!(stub::input("s1", "ls\n").is_ok());
        // new_session yields a valid running session in the contract shape.
        let s = stub::new_session().unwrap();
        assert_eq!(s.status, SessionStatus::Running);
        assert_eq!(s.last_exit, None);
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("id").is_some() && v.get("cwd").is_some());
        assert_eq!(v["status"], json!("running"));
    }

    #[test]
    fn project_serializes_to_the_contract_shape() {
        let p = Project {
            id: "p1".into(),
            name: "arlen".into(),
            path: "/home/x/arlen".into(),
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "id": "p1", "name": "arlen", "path": "/home/x/arlen" })
        );
    }
}
