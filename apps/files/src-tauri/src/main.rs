#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it. The file manager
    // shows names, previews and metadata from files the user did not write, and
    // its backend holds an ambient root capability - so the renderer is exactly
    // the process that should not be able to reach the filesystem itself.
    //
    // Thumbnails already arrive as data URLs from the backend rather than as
    // file paths the renderer opens, so nothing here needs a path granted into
    // the sandbox; if a future preview does, that is the case for the wry-level
    // `WebContext` and `add_path_to_sandbox`, which no environment variable can
    // express.
    //
    // Set before GTK or WebKit initialises, while single-threaded, because
    // `WebContext::set_sandbox_enabled` aborts the process once the webview
    // exists. Left alone if the environment already carries it.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_files_lib::run()
}
