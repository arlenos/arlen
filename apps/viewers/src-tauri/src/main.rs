#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it. The viewers
    // render files the user opened - images, audio, video from anywhere on disk -
    // which is the case this containment exists for: the decoders already run in
    // their own sandbox, and this closes the renderer that displays their output.
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
    if arlen_viewers_lib::handle_default_handler_args() {
        return;
    }
    arlen_viewers_lib::run()
}
