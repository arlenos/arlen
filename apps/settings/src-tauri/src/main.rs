#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Contain the WebKit web process before anything starts it: with the sandbox
    // on, WebKit runs the web process under bwrap and routes its D-Bus through an
    // `xdg-dbus-proxy`. Set here, before GTK or WebKit initialises and while the
    // program is still single-threaded, because `WebContext::set_sandbox_enabled`
    // aborts the process once the webview exists ("Sandboxing cannot be changed
    // after subprocesses were spawned") and Tauri has no earlier hook. Left alone
    // if the environment already carries it, so a deployment or a debugging
    // session can turn it off without a rebuild.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_settings_lib::run()
}
