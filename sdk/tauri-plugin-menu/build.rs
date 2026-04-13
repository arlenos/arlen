fn main() {
    tauri_plugin::Builder::new(&["set_title", "set_breadcrumb", "set_center_content",
        "add_tab", "remove_tab", "update_tab", "activate_tab", "reorder_tabs",
        "add_button", "remove_button", "set_button_enabled", "set_search_mode"])
        .build();
}
