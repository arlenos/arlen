// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A GUI app: no console window on Windows in release. Kept for parity with the
// other apps even though Arlen targets Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it. This app renders
    // the knowledge graph, which is almost entirely content nobody here authored:
    // file names and paths harvested from the filesystem, application names from
    // .desktop files, project names taken from directories, window titles. A
    // hostile string reaches this renderer by being a filename.
    //
    // It is also the app with the least reason to be outside the sandbox: every
    // graph read goes over the daemon socket from the Rust side, so the renderer
    // needs no filesystem, no network and no bus of its own. Nothing here needs a
    // path granted in; if a future preview does, that is the case for the
    // wry-level `WebContext` and `add_path_to_sandbox`, which no environment
    // variable can express.
    //
    // Set before GTK or WebKit initialises, while single-threaded, because
    // `WebContext::set_sandbox_enabled` aborts the process once the webview
    // exists. Left alone if the environment already carries it.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_knowledge_lib::run()
}
