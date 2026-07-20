//! Arlen system monitor: the pure task-manager logic behind the Tauri host - the
//! `/proc` process model + rate mapping ([`procmon`]) and the raw-signal process
//! actions ([`actions`]). No Tauri/webkit deps, so it is unit-tested in CI (the
//! `src-tauri` host that wraps these as commands is not, because it needs the GUI
//! toolchain).

pub mod actions;
pub mod procmon;
