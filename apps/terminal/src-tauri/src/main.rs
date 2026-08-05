#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it. The terminal's
    // renderer displays whatever a command printed, which is content neither the
    // user nor we authored - a program's output is the classic way something
    // hostile reaches a screen.
    //
    // The PTY itself lives in the Rust backend and is reached over Tauri IPC, so
    // the sandbox does not sit between the shell and its terminal; it sits around
    // the process that draws the glyphs.
    //
    // An environment variable rather than `WebContext::set_sandbox_enabled`,
    // which aborts the process when called after the webview exists ("Sandboxing
    // cannot be changed after subprocesses were spawned") and has no earlier hook
    // in Tauri. Set before GTK or WebKit initialises, while single-threaded, and
    // left alone if the environment already carries it so a deployment or a
    // debugging session can turn it off without a rebuild.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_terminal_lib::run()
}
