fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/event.proto");
    println!("cargo:rerun-if-changed=proto/clipboard_api.proto");
    prost_build::compile_protos(
        &["proto/event.proto", "proto/clipboard_api.proto"],
        &["proto/"],
    )?;
    Ok(())
}
