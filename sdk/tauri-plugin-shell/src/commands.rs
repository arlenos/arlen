/// Tauri command handlers for the Lunaris shell plugin.
///
/// Each command is a thin wrapper that takes typed parameters from the
/// Tauri frontend (deserialised by Tauri's command machinery) and
/// forwards them to the matching `os-sdk` shell surface. Errors from
/// the SDK are stringified for the Tauri error channel because Tauri
/// commands cannot return arbitrary Rust error types.

use os_sdk::{
    AnnotationLookup, AnnotationRecord, AnnotationSetParams, PresenceParams, SpatialHint,
    TimelineParams,
};
use tauri::{Runtime, State};

use crate::ShellState;

#[tauri::command]
pub async fn presence_set<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    params: PresenceParams,
) -> Result<(), String> {
    state
        .presence
        .set(params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn presence_clear<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
) -> Result<(), String> {
    state.presence.clear().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn timeline_record<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    params: TimelineParams,
) -> Result<(), String> {
    state
        .timeline
        .record(params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spatial_hint<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    hint: SpatialHint,
) -> Result<(), String> {
    state.spatial.hint(hint).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn annotation_set<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    params: AnnotationSetParams,
) -> Result<(), String> {
    state
        .annotations
        .set(params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn annotation_clear<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    lookup: AnnotationLookup,
) -> Result<(), String> {
    state
        .annotations
        .clear(lookup)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn annotation_get<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    lookup: AnnotationLookup,
) -> Result<Option<AnnotationRecord>, String> {
    state
        .annotations
        .get(lookup)
        .await
        .map_err(|e| e.to_string())
}
