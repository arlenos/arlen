//! Compile the Event Bus proto so installd can emit `permission.changed` at
//! enroll time (E1). Mirrors the per-daemon proto build (knowledge/event-bus).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/event.proto");
    prost_build::compile_protos(&["proto/event.proto"], &["proto/"])?;
    Ok(())
}
