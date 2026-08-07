// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The print dialog's printer list (printing-plan.md PRN-R3).
//!
//! The shell's own commands over the shared `arlen-printers` read, not calls
//! into the Settings panel's: a Tauri command is compiled into one app's binary,
//! so the shell invoking Settings' `printers_list` was rejected at runtime -
//! the dialog would have shown an empty chooser with no error anyone could see.
//!
//! The rows are identical to the panel's by construction (one view crate), which
//! is the point. What differs is the DEFAULT: the panel shows the default
//! printer as a row, the dialog only needs to preselect it, so this returns the
//! name.

/// The configured printer queues, for the dialog's chooser.
#[tauri::command]
pub async fn printers_list() -> Result<Vec<arlen_printers::PrinterView>, String> {
    arlen_printers::list().await
}

/// The default printer's NAME, or `None` when CUPS has none configured.
///
/// A name rather than a row because the dialog uses it to preselect an entry it
/// already has from `printers_list`; sending the whole row again would be a
/// second copy of the same facts for the renderer to keep in step.
#[tauri::command]
pub async fn printers_default() -> Result<Option<String>, String> {
    Ok(arlen_printers::default_printer().await?.map(|p| p.name))
}
