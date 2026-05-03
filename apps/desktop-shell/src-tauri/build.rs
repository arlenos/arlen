fn main() {
    println!("cargo:rerun-if-changed=proto/event.proto");
    println!("cargo:rerun-if-changed=proto/clipboard_api.proto");
    println!("cargo:rerun-if-changed=proto/search_api.proto");
    println!("cargo:rerun-if-changed=proto/intent_api.proto");
    prost_build::compile_protos(
        &[
            "proto/event.proto",
            "proto/clipboard_api.proto",
            "proto/search_api.proto",
            "proto/intent_api.proto",
        ],
        &["proto/"],
    )
    .unwrap();
    tauri_build::build()
}
