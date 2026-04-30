fn main() {
    tauri_plugin::Builder::new(&[
        "pick_file",
        "pick_directory",
        "save_file",
        "save_files",
        "open_uri",
    ])
    .build();
}
