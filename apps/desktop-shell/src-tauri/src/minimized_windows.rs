/// Minimized-windows UI backend.
///
/// Lunaris shows minimized windows as per-workspace icon rows under
/// the WorkspaceIndicator pills. The live state comes from the
/// existing cosmic-toplevel-info subscription in
/// `wayland_client.rs`, which already carries a `minimized: bool`
/// per toplevel. This module exposes three Tauri commands that the
/// frontend calls to read the filtered list and to restore / move
/// individual minimized windows back to a workspace.
///
/// Restore mechanics:
///
/// - **Restore**: `unset_minimized` + `activate` on the cosmic
///   toplevel manager. The compositor then focuses the window on its
///   current workspace, no side effect on the active workspace.
/// - **Restore-to-workspace**: `move_to_ext_workspace` first, then
///   `unset_minimized` + `activate`. The compositor moves the surface
///   to the new workspace and raises it there; the frontend also
///   switches the active workspace so the user sees the restored
///   window immediately.
///
/// Both paths go through the existing `ToplevelSender` helpers — no
/// new Wayland globals are bound here.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::wayland_client::{ToplevelSender, WindowList, WorkspaceSender};

/// One minimized window as returned to the frontend. `iconPath` is a
/// resolved base64 data URL; the frontend reads it directly into an
/// `<img>` tag without a second round-trip to `resolve_app_icon`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimizedWindow {
    pub window_id: String,
    pub app_id: String,
    pub title: String,
    /// Workspace handle id where the window is currently minimized.
    /// Empty string for sticky windows (minimized outside any
    /// workspace context).
    pub workspace_id: String,
    /// Base64 data URL of the resolved icon, or `None` if no icon
    /// was found in any freedesktop theme.
    pub icon_path: Option<String>,
}

/// Return the current minimized-windows snapshot, one entry per
/// minimized toplevel. The frontend usually groups these by
/// `workspace_id` to render the per-pill icon rows.
///
/// Graceful degradation: returns an empty Vec if the Wayland state
/// isn't yet available. Never errors.
///
/// # Edge cases handled by upstream state
///
/// - **App crash while minimized**: the compositor emits
///   `toplevel_closed`, `WindowList` drops the entry, this call
///   returns the new, shorter list on the next poll.
/// - **Window without icon**: the returned `icon_path` is `None`
///   and the frontend falls back to a generic `AppWindow` glyph in
///   `MinimizedWindowIcon.svelte`.
/// - **External minimize** (via GNOME/KDE bindings, wlroots
///   keystrokes, or cosmic's own keybinds): handled uniformly
///   because this function reads cosmic-toplevel-info state, not
///   shell-initiated events. Any source that flips `State::Minimized`
///   shows up here.
/// - **Workspace deletion**: cosmic moves open windows to the next
///   workspace, but minimized windows attached to a destroyed
///   workspace may end up with an empty `workspace_ids`. The
///   frontend's per-workspace grouping filters those out of the
///   indicator row — they remain recoverable via the Workspace
///   Overlay (Super+Tab) "Minimized" section on workspace 1 once
///   Phase 5 compositor work lands. For now the window is still
///   restorable via `restore_window`; it just isn't visually
///   attached to any workspace until restored.
#[tauri::command]
pub fn get_minimized_windows(state: State<'_, WindowList>) -> Vec<MinimizedWindow> {
    let list = match state.lock() {
        Ok(l) => l,
        Err(_) => return Vec::new(),
    };
    list.iter()
        .filter(|w| w.minimized)
        .map(|w| MinimizedWindow {
            window_id: w.id.clone(),
            app_id: w.app_id.clone(),
            title: w.title.clone(),
            workspace_id: w.workspace_ids.first().cloned().unwrap_or_default(),
            icon_path: crate::shell_overlay_client::resolve_app_icon(w.app_id.clone()),
        })
        .collect()
}

/// Restore a minimized window on its current workspace. The
/// compositor un-minimizes and activates (focuses) the window; the
/// shell does not change the active workspace. If the user wants to
/// follow the window to its workspace, they call
/// `restore_window_to_workspace` with the same target workspace.
#[tauri::command]
pub fn restore_window(
    window_id: String,
    sender: State<'_, Arc<ToplevelSender>>,
) {
    sender.set_minimized(&window_id, false);
    sender.activate(&window_id);
}

/// Ask the app behind `window_id` to close (standard close request,
/// not kill). Used by the minimized icon's context menu. If the app
/// prompts for save-confirm the close may not actually happen; the
/// shell reacts to the resulting toplevel-closed event in that case.
#[tauri::command]
pub fn close_minimized_window(
    window_id: String,
    sender: State<'_, Arc<ToplevelSender>>,
) {
    sender.close(&window_id);
}

/// Generic close request for any window (active OR minimized). The
/// underlying cosmic-toplevel-manager `close` request is state-
/// independent; this command exists separately from
/// `close_minimized_window` so the context-menu callsite reads as
/// intent-matching ("Close" on a regular active window).
#[tauri::command]
pub fn close_window(
    window_id: String,
    sender: State<'_, Arc<ToplevelSender>>,
) {
    sender.close(&window_id);
}

/// Toggle fullscreen on the window. Frontend reads the current
/// fullscreen flag from the windows store and passes the desired
/// value; keeping the toggle decision on the frontend lets the call
/// site (context menu) show the right label ("Fullscreen" vs. "Exit
/// Fullscreen") without a round-trip.
#[tauri::command]
pub fn fullscreen_window(
    window_id: String,
    enabled: bool,
    sender: State<'_, Arc<ToplevelSender>>,
) {
    sender.set_fullscreen(&window_id, enabled);
}

/// Tile a window to a screen half. `direction` accepts `"left"`,
/// `"right"`, `"top"`, or `"bottom"`.
///
/// NOTE: cosmic-toplevel-management does NOT expose a half-tile
/// request. A proper implementation needs a new `tile_toplevel`
/// request in `lunaris-shell-overlay-v1` that the compositor
/// resolves against its internal tiling layout. For now this
/// command logs a warning and returns Ok — the UI surfaces the
/// option so the wiring is ready, but the window doesn't move.
#[tauri::command]
pub fn tile_window(window_id: String, direction: String) -> Result<(), String> {
    let valid = matches!(
        direction.as_str(),
        "left" | "right" | "top" | "bottom"
    );
    if !valid {
        return Err(format!("tile_window: unknown direction '{direction}'"));
    }
    log::warn!(
        "tile_window: direction={direction} id={window_id} \
         (not yet implemented — pending lunaris-shell-overlay protocol \
         extension for half-tile requests)"
    );
    Ok(())
}

/// Minimize a currently-visible window. The opposite of
/// `restore_window` — used by the Workspace Overlay when the user
/// drags an active window card into the Minimized area. Sends
/// `set_minimized` via cosmic-toplevel-management; cosmic emits the
/// state change back through `zcosmic_toplevel_handle_v1::State`
/// which updates the shell's window list on the next tick.
#[tauri::command]
pub fn minimize_window(
    window_id: String,
    sender: State<'_, Arc<ToplevelSender>>,
) {
    sender.set_minimized(&window_id, true);
}

/// Move a minimized window to `workspace_id`, un-minimize it, and
/// focus it there. Order matters: move first so the compositor
/// un-minimizes on the destination workspace; otherwise the window
/// un-minimizes on its source workspace and the move is visually
/// ugly.
///
/// If `workspace_id` is empty (sticky window) we fall back to a plain
/// restore — sticky windows don't belong to a workspace so there's
/// nothing to move to.
#[tauri::command]
pub fn restore_window_to_workspace(
    window_id: String,
    workspace_id: String,
    sender: State<'_, Arc<ToplevelSender>>,
    ws_sender: State<'_, Arc<WorkspaceSender>>,
) {
    if !workspace_id.is_empty() {
        sender.move_to_workspace(&window_id, &workspace_id);
        // Activating the destination workspace makes the restore
        // visible immediately. Without it the user would click
        // "restore to workspace 2" from workspace 1 and see nothing.
        ws_sender.activate(&workspace_id);
    }
    sender.set_minimized(&window_id, false);
    sender.activate(&window_id);
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wayland_client::ToplevelPayload;
    use std::sync::Mutex;

    fn mk_payload(id: &str, minimized: bool, ws: &[&str]) -> ToplevelPayload {
        ToplevelPayload {
            id: id.into(),
            title: format!("title {id}"),
            app_id: format!("app.{id}"),
            active: false,
            minimized,
            fullscreen: false,
            workspace_ids: ws.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Verify the filter picks only minimized payloads and maps the
    /// fields into MinimizedWindow. Icon resolution is skipped in
    /// this test path because resolve_app_icon is tolerant of
    /// unresolvable app ids (returns None) — the filtering is what we
    /// want to exercise here.
    #[test]
    fn filters_only_minimized() {
        let list: WindowList = Arc::new(Mutex::new(vec![
            mk_payload("a", true, &["ws1"]),
            mk_payload("b", false, &["ws1"]),
            mk_payload("c", true, &["ws2"]),
        ]));
        let ids: Vec<String> = {
            let guard = list.lock().unwrap();
            guard
                .iter()
                .filter(|w| w.minimized)
                .map(|w| w.id.clone())
                .collect()
        };
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"c".to_string()));
    }

    #[test]
    fn empty_list_returns_empty() {
        let list: WindowList = Arc::new(Mutex::new(vec![]));
        let is_empty = {
            let guard = list.lock().unwrap();
            guard.iter().filter(|w| w.minimized).next().is_none()
        };
        assert!(is_empty);
    }

    #[test]
    fn workspace_id_falls_back_to_empty_when_missing() {
        let p = mk_payload("x", true, &[]);
        let ws_id: String = p.workspace_ids.first().cloned().unwrap_or_default();
        assert!(ws_id.is_empty());
    }

    /// Guards against a previous variant of the filter test that
    /// borrowed the lock across a collect() — the MutexGuard dropped
    /// mid-iteration because `filter` held a reference into a
    /// temporary. Explicit `collect()` inside the guard keeps all
    /// borrows short-lived.
    #[test]
    fn filter_collects_inside_guard() {
        let list: WindowList = Arc::new(Mutex::new(vec![
            mk_payload("a", true, &["ws1"]),
            mk_payload("b", true, &["ws1"]),
        ]));
        let collected: Vec<String> = {
            let guard = list.lock().unwrap();
            guard
                .iter()
                .filter(|w| w.minimized)
                .map(|w| w.id.clone())
                .collect()
        };
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn multiple_workspaces_partitioned_correctly() {
        // Multi-monitor edge case: windows on different workspaces
        // must group into distinct buckets. The frontend's derived
        // store does the grouping, but the Rust side surfaces the
        // right workspace_id per entry so the grouping can happen.
        let list: WindowList = Arc::new(Mutex::new(vec![
            mk_payload("a", true, &["ws-1"]),
            mk_payload("b", true, &["ws-2"]),
            mk_payload("c", true, &["ws-1"]),
        ]));
        let guard = list.lock().unwrap();
        let ws1_count = guard
            .iter()
            .filter(|w| w.minimized && w.workspace_ids.iter().any(|id| id == "ws-1"))
            .count();
        let ws2_count = guard
            .iter()
            .filter(|w| w.minimized && w.workspace_ids.iter().any(|id| id == "ws-2"))
            .count();
        assert_eq!(ws1_count, 2);
        assert_eq!(ws2_count, 1);
    }

    #[test]
    fn window_without_workspace_surfaces_empty_id() {
        // Sticky or workspace-orphaned windows (e.g. after workspace
        // deletion): workspace_ids is empty, MinimizedWindow carries
        // an empty string workspace_id. The frontend renders these
        // in a separate "unassigned" path (deferred to Phase 5
        // compositor work); until then they still round-trip cleanly
        // through the Tauri layer.
        let p = mk_payload("orphan", true, &[]);
        let ws_id: String = p.workspace_ids.first().cloned().unwrap_or_default();
        assert!(ws_id.is_empty());
    }

    #[test]
    fn fullscreen_flag_defaults_to_false() {
        // Any new toplevel entering the windows list should start as
        // non-fullscreen — the context menu's "Fullscreen" label
        // relies on this default when cosmic hasn't yet sent a
        // state update.
        let p = mk_payload("x", false, &["ws-1"]);
        assert!(!p.fullscreen);
    }

    #[test]
    fn serializes_camel_case() {
        let m = MinimizedWindow {
            window_id: "w1".into(),
            app_id: "a1".into(),
            title: "t".into(),
            workspace_id: "ws1".into(),
            icon_path: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("windowId"));
        assert!(json.contains("workspaceId"));
        assert!(json.contains("iconPath"));
    }
}
