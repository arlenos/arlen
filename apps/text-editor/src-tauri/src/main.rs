// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A GUI app: no console window on Windows in release. Kept for parity with the
// other apps even though Arlen targets Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it. This renderer
    // displays the contents of whatever file the user opened - text nobody here
    // authored, from anywhere on disk - which is precisely the case the sandbox
    // exists for.
    //
    // Set before GTK or WebKit initialises, while single-threaded, because
    // `WebContext::set_sandbox_enabled` aborts the process once the webview
    // exists. Left alone if the environment already carries it.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_text_editor_lib::run()
}
