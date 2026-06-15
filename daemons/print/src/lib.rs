//! Arlen CUPS/IPP integration backend (printing-plan.md PRN-R1).
//!
//! The print stack is CUPS, driverless-first (IPP Everywhere / AirPrint): a
//! network printer advertises over DNS-SD and accepts IPP with no driver. This
//! crate is the small backend surface over CUPS the rest of Arlen calls:
//!
//! - [`model`] - the typed printer / job / state model and the local-vs-network
//!   destination classification (the print-as-egress angle, §4.2);
//! - [`backend`] - the [`backend::PrintBackend`] trait (enumerate / submit /
//!   query / cancel) plus a mock, so the portal (PRN-R2) and the Settings panel
//!   (PRN-R4) are testable without a print server;
//! - [`cups`] - the real IPP client to cupsd, behind the `cups` feature and
//!   verified on hardware;
//! - [`audit`] - the coarse, content-free print-audit (a network print records
//!   as a network call, a local one as a permission event).
//!
//! Printing is never gated (it is a user-driven action); the portal isolation +
//! the audit are the security posture (printing-plan.md §4).

pub mod audit;
pub mod backend;
#[cfg(feature = "cups")]
pub mod cups;
pub mod model;

pub use audit::print_audit_event;
pub use backend::{ColorMode, Duplex, JobOptions, PrintBackend, PrintError, PrintSubmission};
#[cfg(feature = "cups")]
pub use cups::CupsBackend;
pub use model::{classify_destination, Destination, Job, JobState, Printer, PrinterState};
