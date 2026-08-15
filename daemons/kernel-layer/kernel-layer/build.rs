use aya_build::{Package, Toolchain};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile proto types for use in normalizer.rs
    prost_build::compile_protos(&["proto/event.proto"], &["proto/"])?;

    // Rerun when the proto changes. Emitting any rerun-if-changed narrows the build
    // script's triggers to exactly the listed files, so the proto must be named
    // explicitly or a schema edit never regenerates the types.
    println!("cargo:rerun-if-changed=proto/event.proto");

    // Tell Cargo to rerun if eBPF source changes
    println!("cargo:rerun-if-changed=../kernel-layer-ebpf/src/main.rs");

    // Note: aya-build 0.1.3 has a naming conflict when package == binary name.
    // We trigger the eBPF build manually via the justfile instead.
    let _ = (Package::default(), Toolchain::default()); // suppress unused import warnings

    // WHERE THE eBPF OBJECT ACTUALLY IS, told to the compiler rather than guessed
    // by a relative path.
    //
    // `main.rs` used to include it as `../../target/bpfel-unknown-none/release/...`,
    // which assumes the target directory sits beside this crate. The repo's
    // `.cargo/config.toml` sets `target-dir = "target"`, so cargo puts it at the
    // REPO root instead, and the include failed with `No such file or directory`
    // on a tree where the object had just been built successfully.
    //
    // Searching upward from OUT_DIR finds it under either layout, and emitting the
    // absolute path means the include cannot drift again if the layout changes.
    // Absent is not an error here: the eBPF crate is built by a separate cargo
    // invocation, so a first pass legitimately runs before it exists, and the
    // include's own error is the right place for that to surface.
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let rel = "bpfel-unknown-none/release/kernel-layer-ebpf";
        let mut dir = std::path::PathBuf::from(&out_dir);
        while dir.pop() {
            let candidate = dir.join(rel);
            if candidate.is_file() {
                println!("cargo:rustc-env=ARLEN_EBPF_OBJECT={}", candidate.display());
                println!("cargo:rerun-if-changed={}", candidate.display());
                break;
            }
        }
    }
    Ok(())
}
