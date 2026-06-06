fn main() {
    tauri_plugin::Builder::new(&[
        "write",
        "read",
        "history",
        "subscribe",
        "unsubscribe",
    ])
    .build();
}
