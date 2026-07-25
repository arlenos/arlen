// Prevents an extra console window on Windows in release; noop on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    arlen_screenshot_lib::run()
}
