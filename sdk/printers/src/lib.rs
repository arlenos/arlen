// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The printer reads more than one surface needs.
//!
//! Two surfaces list printers: the Settings panel that configures them and the
//! shell's print dialog that picks one. They were reading through the same
//! command name, which does not work - a Tauri command is compiled into one
//! app's binary, so the shell's call into Settings' `printers_list` was rejected
//! at runtime. The fix is not to give the shell a private copy of the mapping;
//! it is to put the read where both can have it.
//!
//! `arlen-print` already owns the IPP conversation. What was app-local was the
//! WIRE SHAPE - `arlen_print::Printer` is deliberately not serializable, so each
//! surface was about to invent its own view of the same rows. This crate is that
//! view, once.
//!
//! Interim by design and not wasted: `printing-plan.md` PRN-R1 makes CUPS a
//! service surface, and its daemon wraps the same read. Until then this kills
//! the cross-app call without removing the shell's printer list to prove a
//! point.

use arlen_print::{CupsBackend, PrintBackend, Printer};
use serde::Serialize;

/// A printer as a surface lists it.
///
/// camelCase on the wire because both consumers are TypeScript, and the state
/// and destination are their stable lowercase keys rather than Debug output, so
/// a renderer can switch on them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrinterView {
    pub name: String,
    pub uri: String,
    pub info: Option<String>,
    pub location: Option<String>,
    pub make_model: Option<String>,
    pub state: String,
    pub accepting_jobs: bool,
    pub destination: String,
}

impl From<Printer> for PrinterView {
    fn from(p: Printer) -> Self {
        Self {
            name: p.name,
            uri: p.uri,
            info: p.info,
            location: p.location,
            make_model: p.make_model,
            state: p.state.as_key().to_string(),
            accepting_jobs: p.accepting_jobs,
            destination: p.destination.as_key().to_string(),
        }
    }
}

/// The configured printer queues on the local CUPS server.
///
/// The error is a string because both callers are Tauri commands whose failure
/// reaches a surface as text; nothing here needs to match on the cause.
pub async fn list() -> Result<Vec<PrinterView>, String> {
    CupsBackend::default()
        .printers()
        .await
        .map(|ps| ps.into_iter().map(PrinterView::from).collect())
        .map_err(|e| e.to_string())
}

/// The default printer, or `None` when CUPS has none configured.
pub async fn default_printer() -> Result<Option<PrinterView>, String> {
    CupsBackend::default()
        .default_printer()
        .await
        .map(|p| p.map(PrinterView::from))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_shape_is_the_one_both_surfaces_declare() {
        // Both consumers' TypeScript declares camelCase keys and lowercase state
        // strings. Pinning it here is what keeps one view from drifting into two.
        let v = PrinterView {
            name: "office".into(),
            uri: "ipp://printer.local/ipp/print".into(),
            info: None,
            location: Some("Floor 2".into()),
            make_model: Some("Acme 9000".into()),
            state: "idle".into(),
            accepting_jobs: true,
            destination: "network".into(),
        };
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["makeModel"], "Acme 9000");
        assert_eq!(json["acceptingJobs"], true);
        assert_eq!(json["state"], "idle");
        // An absent field is null rather than missing, so a renderer can tell
        // "no info recorded" from a shape it did not expect.
        assert!(json.get("info").is_some() && json["info"].is_null());
    }
}
