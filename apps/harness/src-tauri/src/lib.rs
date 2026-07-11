//! Arlen AI harness app backend.
//!
//! Tauri entry point for the GUI door to the AI layer (conversation +
//! agent observability). A1 is the skeleton: a runnable window and the
//! frontend-log bridge. The daemon (query/chat), agent D-Bus, audit
//! read, and Event Bus wiring land in A2+ (see
//! `docs/architecture/ai-app.md` §7).

mod activity;
mod ai_client;
mod capsule;
mod drive;
mod ai_manage;
mod app_meta;
mod behaviours;
mod capability;
mod file_ref;
mod mention;
mod notices;
mod prep;
mod pins;
mod save;
mod sessions;
mod url;

/// Route a log line from the frontend into the Rust logger so it shows
/// up in the same stdout stream as backend logs. Tauri WebView DevTools
/// are not always reachable, so frontend diagnostics go through here.
#[tauri::command]
fn frontend_log(level: String, msg: String) {
    match level.as_str() {
        "warn" => log::warn!("[frontend] {msg}"),
        "error" => log::error!("[frontend] {msg}"),
        _ => log::info!("[frontend] {msg}"),
    }
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            frontend_log,
            ai_client::ai_query,
            drive::pi_prompt,
            ai_client::ai_explain,
            activity::ai_activity_recent,
            activity::ai_reads_recent,
            ai_manage::ai_models_list,
            ai_manage::ai_active,
            ai_manage::ai_set_active,
            ai_manage::ai_set_action_mode,
            ai_manage::ai_set_autonomous_app,
            ai_manage::ai_usage,
            ai_manage::ai_providers_list,
            ai_manage::ai_provider_set_enabled,
            ai_manage::ai_provider_test,
            ai_manage::ai_defaults_get,
            ai_manage::open_ai_settings,
            ai_manage::ai_working_set,
            ai_manage::pending_proposals,
            ai_manage::completed_actions,
            ai_manage::ai_access_grants,
            app_meta::app_metadata,
            ai_manage::deny,
            ai_manage::approve,
            ai_manage::undo_action,
            ai_manage::action_state,
            capability::ai_capability,
            behaviours::ai_behaviours,
            notices::ai_notices,
            mention::list_files,
            mention::read_mention_file,
            file_ref::fileref_resolve,
            file_ref::fileref_open,
            file_ref::fileref_reveal,
            save::artifact_save,
            pins::artifact_pins_load,
            pins::artifact_pins_save,
            sessions::harness_sessions_load,
            sessions::harness_sessions_save,
            capsule::capsule_scope_options,
            capsule::capsule_preview,
            capsule::capsule_mint,
            prep::prep_for,
            url::open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-harness");
}
