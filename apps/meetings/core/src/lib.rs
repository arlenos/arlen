//! The pure Arlen meetings core: logic the Tauri host wraps as commands,
//! unit-tested in CI (the `src-tauri` host is not, its webkit deps keep it off
//! the runners).
//!
//! Today this is the on-disk note store: the verifiable AI meeting note written
//! by the summarize-and-file flow and read back by the app, with a save/load
//! round-trip guard.
pub mod note_store;
