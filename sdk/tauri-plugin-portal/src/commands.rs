//! Tauri commands. Thin wrappers over the `api::*` functions so
//! the same code drives both `invoke()` from JS and direct Rust
//! callers (`app-settings`).

use crate::api;
use crate::types::{
    OpenUriOptions, PickFileOptions, PickerError, PickerResult, SaveFileOptions,
    SaveFilesOptions,
};

#[tauri::command]
pub async fn pick_file(options: PickFileOptions) -> Result<PickerResult, PickerError> {
    api::pick_file(options).await
}

#[tauri::command]
pub async fn pick_directory(
    options: PickFileOptions,
) -> Result<PickerResult, PickerError> {
    api::pick_directory(options).await
}

#[tauri::command]
pub async fn save_file(options: SaveFileOptions) -> Result<PickerResult, PickerError> {
    api::save_file(options).await
}

#[tauri::command]
pub async fn save_files(options: SaveFilesOptions) -> Result<PickerResult, PickerError> {
    api::save_files(options).await
}

#[tauri::command]
pub async fn open_uri(uri: String, options: OpenUriOptions) -> Result<(), PickerError> {
    api::open_uri(&uri, options).await
}
