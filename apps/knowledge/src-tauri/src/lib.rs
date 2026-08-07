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

mod projects;

/// Build and run the app.
///
/// # Panics
/// If the Tauri runtime cannot start.
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![
            projects::knowledge_list,
            projects::knowledge_projects_list
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Knowledge app");
}
