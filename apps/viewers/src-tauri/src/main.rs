#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if arlen_viewers_lib::handle_default_handler_args() {
        return;
    }
    arlen_viewers_lib::run()
}
