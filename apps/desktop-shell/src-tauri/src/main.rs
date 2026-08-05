// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it. The shell renders
    // window titles, notification bodies and tray tooltips that other programs
    // supplied, so its renderer handles content from every app on the machine
    // while sitting in the process that owns the topbar.
    //
    // Set before GTK or WebKit initialises, while single-threaded, because
    // `WebContext::set_sandbox_enabled` aborts the process once a webview exists
    // ("Sandboxing cannot be changed after subprocesses were spawned") and Tauri
    // has no earlier hook. This is also why it is here rather than in
    // `layer_shell::init`, which runs per window, long after the first web
    // process is up.
    //
    // Left alone if the environment already carries it, so a deployment or a
    // debugging session can turn it off without a rebuild.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_desktop_shell_lib::run()
}
