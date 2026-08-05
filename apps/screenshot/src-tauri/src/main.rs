// Prevents an extra console window on Windows in release; noop on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it.
    //
    // The renderer displays an image the user just captured, and a compromised
    // web process is otherwise inside the session with the app's own reach. With
    // this set, WebKit runs the web process under bwrap and routes its D-Bus
    // through an `xdg-dbus-proxy`, both verified in the process tree.
    //
    // An environment variable rather than `WebContext::set_sandbox_enabled`,
    // which is not merely unavailable but fatal from an app: calling it once the
    // webview exists aborts the process with "Sandboxing cannot be changed after
    // subprocesses were spawned", and Tauri offers no hook that runs earlier.
    // Set here, in `main`, before GTK or WebKit has initialised anything and
    // while the program is still single-threaded.
    //
    // Left as-is if the environment already carries it, so a deployment or a
    // debugging session can turn it off without a rebuild.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_screenshot_lib::run()
}
