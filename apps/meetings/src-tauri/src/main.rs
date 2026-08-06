#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The renderer is sandboxed. WebKitGTK enables its bwrap sandbox by default
    // in the versions we ship, and this makes that explicit rather than inherited:
    // an app should state the containment it runs under, not depend on a distro
    // default staying true. `WEBKIT_FORCE_SANDBOX` can only turn it ON in current
    // WebKit - the escape hatch is a separate, deliberately alarming variable.
    //
    // Set before GTK or WebKit initialises, while single-threaded, because
    // `WebContext::set_sandbox_enabled` aborts the process once the webview
    // exists. Left alone if the environment already carries it.
    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    arlen_meetings_lib::run()
}
