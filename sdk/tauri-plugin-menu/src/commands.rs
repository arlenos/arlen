/// Tauri commands for the titlebar protocol.
///
/// Each command writes a request to the Wayland titlebar object and
/// flushes the connection. The commands are registered by the plugin
/// and callable from the TypeScript frontend.

use serde::Deserialize;
use tauri::{command, State};

use crate::wayland::SharedConnection;

/// Flush the Wayland connection after sending a request.
fn flush(shared: &SharedConnection) {
    let lock = shared.lock().unwrap();
    if let Some(ref conn) = lock.conn {
        let _ = conn.flush();
    }
}

/// Check that the titlebar object is bound.
fn with_titlebar<F>(shared: &SharedConnection, f: F) -> Result<(), String>
where
    F: FnOnce(&crate::protocol::lunaris_titlebar_v1::LunarisTitlebarV1),
{
    let lock = shared.lock().unwrap();
    let tb = lock
        .titlebar
        .as_ref()
        .ok_or_else(|| "titlebar not bound (no surface registered)".to_string())?;
    f(tb);
    drop(lock);
    flush(shared);
    Ok(())
}

#[command]
pub async fn set_title(
    shared: State<'_, SharedConnection>,
    title: String,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.set_title(title))
}

#[command]
pub async fn set_breadcrumb(
    shared: State<'_, SharedConnection>,
    segments_json: String,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.set_breadcrumb(segments_json))
}

#[command]
pub async fn set_center_content(
    shared: State<'_, SharedConnection>,
    content: u32,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| {
        let mode = match content {
            1 => crate::protocol::lunaris_titlebar_v1::CenterContent::Tabs,
            2 => crate::protocol::lunaris_titlebar_v1::CenterContent::Search,
            3 => crate::protocol::lunaris_titlebar_v1::CenterContent::Segmented,
            _ => crate::protocol::lunaris_titlebar_v1::CenterContent::None,
        };
        tb.set_center_content(mode);
    })
}

#[derive(Deserialize)]
pub struct TabInfo {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub status: Option<u32>,
}

#[command]
pub async fn add_tab(
    shared: State<'_, SharedConnection>,
    tab: TabInfo,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| {
        let status = match tab.status.unwrap_or(0) {
            1 => crate::protocol::lunaris_titlebar_v1::TabStatus::Modified,
            2 => crate::protocol::lunaris_titlebar_v1::TabStatus::Pinned,
            _ => crate::protocol::lunaris_titlebar_v1::TabStatus::Normal,
        };
        tb.add_tab(tab.id, tab.title, tab.icon, status);
    })
}

#[command]
pub async fn remove_tab(
    shared: State<'_, SharedConnection>,
    id: String,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.remove_tab(id))
}

#[command]
pub async fn update_tab(
    shared: State<'_, SharedConnection>,
    id: String,
    title: String,
    status: Option<u32>,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| {
        let s = match status.unwrap_or(0) {
            1 => crate::protocol::lunaris_titlebar_v1::TabStatus::Modified,
            2 => crate::protocol::lunaris_titlebar_v1::TabStatus::Pinned,
            _ => crate::protocol::lunaris_titlebar_v1::TabStatus::Normal,
        };
        tb.update_tab(id, title, s);
    })
}

#[command]
pub async fn activate_tab(
    shared: State<'_, SharedConnection>,
    id: String,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.activate_tab(id))
}

#[command]
pub async fn reorder_tabs(
    shared: State<'_, SharedConnection>,
    ids_json: String,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.reorder_tabs(ids_json))
}

#[command]
pub async fn add_button(
    shared: State<'_, SharedConnection>,
    id: String,
    icon: String,
    tooltip: String,
    position: Option<u32>,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| {
        let pos = match position.unwrap_or(1) {
            0 => crate::protocol::lunaris_titlebar_v1::ButtonPosition::Left,
            _ => crate::protocol::lunaris_titlebar_v1::ButtonPosition::Right,
        };
        tb.add_button(id, icon, tooltip, pos);
    })
}

#[command]
pub async fn remove_button(
    shared: State<'_, SharedConnection>,
    id: String,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.remove_button(id))
}

#[command]
pub async fn set_button_enabled(
    shared: State<'_, SharedConnection>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.set_button_enabled(id, if enabled { 1 } else { 0 }))
}

#[command]
pub async fn set_search_mode(
    shared: State<'_, SharedConnection>,
    enabled: bool,
) -> Result<(), String> {
    with_titlebar(&shared, |tb| tb.set_search_mode(if enabled { 1 } else { 0 }))
}
