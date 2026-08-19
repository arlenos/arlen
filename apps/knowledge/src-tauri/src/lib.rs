// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Knowledge app's Rust side: the scoped reads its browser rides.
//!
//! The app is a WINDOW onto the knowledge daemon's graph, never a second store.
//! Every command here is a read through the daemon's caller-scoped socket, so
//! what this app can see is exactly what its own permission profile grants -
//! opening the window cannot widen it.
//!
//! Each command degrades to an error rather than to plausible data. The
//! frontend answers a failed read with its fixture and says so (`*Mocked`
//! stores), which is only honest while a failure is really a failure: a command
//! that invented an empty list would show the fixture's absence as fact.

mod export;
mod delete;
mod pause;
mod projects;
mod settings_link;
mod provenance;
mod report;
mod search;
mod service;
mod searches;
mod timeline;

/// Build and run the app.
///
/// # Panics
/// If the Tauri runtime cannot start.
pub fn run() {
    // This app at info, dependencies at warn, and both halves are a fix.
    //
    // A bare `env_logger::init()` defaults to `error`, so every `log::info!`
    // and `log::warn!` here produced nothing: the app was mute in the journal.
    // That is the failure that made the boot consent hang so hard to find -
    // the component in the middle could not be heard - and it was true of four
    // apps at once.
    //
    // Dependencies stay at warn rather than being swept up to info with it,
    // because zbus logs D-Bus handshake frames WITH their message bytes, and a
    // message body is user content: paths, queries, notification text. At info
    // that lands in a journal no capability grant covers. `RUST_LOG=zbus=trace`
    // still gets it, deliberately.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,arlen_knowledge_lib=info"),
    )
    .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![
            projects::knowledge_list,
            projects::knowledge_projects_list,
            provenance::knowledge_provenance,
            search::knowledge_search,
            search::knowledge_project_names,
            timeline::knowledge_timeline,
            export::knowledge_timeline_export,
            delete::knowledge_timeline_delete,
            pause::knowledge_timeline_pause,
            pause::knowledge_refresh_interval_ms,
            pause::knowledge_timeline_paused,
            settings_link::open_settings_route,
            searches::knowledge_searches,
            searches::knowledge_search_save
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Knowledge app");
}
