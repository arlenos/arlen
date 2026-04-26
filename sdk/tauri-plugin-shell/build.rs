fn main() {
    tauri_plugin::Builder::new(&[
        "presence_set",
        "presence_clear",
        "timeline_record",
        "spatial_hint",
        "annotation_set",
        "annotation_clear",
        "annotation_get",
    ])
    .build();
}
