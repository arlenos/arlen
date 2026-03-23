fn main() {
    prost_build::compile_protos(&["proto/event.proto"], &["proto/"]).unwrap();
    tauri_build::build()
}
