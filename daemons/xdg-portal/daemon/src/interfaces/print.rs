//! The `org.freedesktop.impl.portal.Print` backend (printing-plan.md PRN-R2).
//!
//! Bridges a portal print request to the built `arlen-print` service (CUPS,
//! PRN-R1). This module holds the pure mapping from the portal's print settings
//! (`a{sv}`, the GTK/CUPS vocabulary) to `arlen_print` [`JobOptions`]; the
//! interface impl (`PreparePrint` / `Print` + document-fd handling, the Request
//! pattern) builds on it.

use std::collections::HashMap;
use std::sync::Arc;

use arlen_print::backend::{ColorMode, Duplex, JobOptions, PrintBackend, PrintError, PrintSubmission};
use arlen_print::cups::CupsBackend;
use arlen_print::service::PrintService;
use audit_proto::sink::LedgerAuditSink;
use zbus::zvariant::{OwnedValue, Value};

/// Read a string setting from the portal's `a{sv}` settings map. GTK print
/// settings carry their values as strings (e.g. `n-copies = "2"`).
fn setting_str(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match Value::try_from(settings.get(key)?.clone()).ok()? {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Map the portal print settings (`a{sv}`) to [`JobOptions`]. Absent or
/// unrecognised keys leave the option `None` (the print server's own default).
/// Recognised keys: `copies` / `n-copies`, `sides` (duplex), `print-color-mode`,
/// `media`.
pub fn job_options_from_settings(settings: &HashMap<String, OwnedValue>) -> JobOptions {
    let copies = setting_str(settings, "copies")
        .or_else(|| setting_str(settings, "n-copies"))
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&n| n >= 1);

    let duplex = setting_str(settings, "sides").and_then(|s| match s.as_str() {
        "one-sided" => Some(Duplex::OneSided),
        "two-sided-long-edge" => Some(Duplex::TwoSidedLongEdge),
        "two-sided-short-edge" => Some(Duplex::TwoSidedShortEdge),
        _ => None,
    });

    let color = setting_str(settings, "print-color-mode").and_then(|s| match s.as_str() {
        "color" => Some(ColorMode::Color),
        "monochrome" | "auto-monochrome" | "process-monochrome" => Some(ColorMode::Monochrome),
        _ => None,
    });

    let media = setting_str(settings, "media").filter(|s| !s.is_empty());

    JobOptions {
        copies,
        duplex,
        color,
        media,
    }
}

#[allow(dead_code)] // wired by the #[interface] impl next increment
/// The `org.freedesktop.impl.portal.Print` backend state: the arlen-print service
/// over the CUPS print system, recording submits to the audit ledger (the printer
/// + destination, never the document).
pub struct Print {
    service: PrintService<CupsBackend>,
}

impl Print {
    /// Construct the backend over CUPS + the default audit ledger socket.
    pub fn new() -> Self {
        Self {
            service: PrintService::new(
                CupsBackend::default(),
                Arc::new(LedgerAuditSink::at_default_socket()),
            ),
        }
    }
}

impl Default for Print {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
/// Submit a document to the default printer with the portal settings mapped to
/// job options - the bridge the impl.portal.Print `Print` method calls once it has
/// read the document fd. Fails closed if no printer is configured (never silently
/// drops the job). Generic over the backend so it is testable with a mock.
async fn submit_document<B: PrintBackend>(
    service: &PrintService<B>,
    app_id: &str,
    document: &[u8],
    title: Option<&str>,
    settings: &HashMap<String, OwnedValue>,
) -> Result<i32, PrintError> {
    let printer = service
        .default_printer()
        .await?
        .ok_or_else(|| PrintError::Invalid("no default printer configured".to_string()))?;
    let submission = PrintSubmission {
        printer: &printer.name,
        document,
        title,
        mime: None,
        options: job_options_from_settings(settings),
    };
    // `app_id` is the calling app: the audit records it as the acting principal.
    service.submit(app_id, &submission).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(v: Value<'static>) -> OwnedValue {
        OwnedValue::try_from(v).unwrap()
    }

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, OwnedValue> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), owned(Value::from(v.to_string()))))
            .collect()
    }

    #[test]
    fn maps_the_recognised_settings() {
        let s = settings(&[
            ("n-copies", "3"),
            ("sides", "two-sided-long-edge"),
            ("print-color-mode", "monochrome"),
            ("media", "iso_a4_210x297mm"),
        ]);
        let o = job_options_from_settings(&s);
        assert_eq!(o.copies, Some(3));
        assert!(matches!(o.duplex, Some(Duplex::TwoSidedLongEdge)));
        assert!(matches!(o.color, Some(ColorMode::Monochrome)));
        assert_eq!(o.media.as_deref(), Some("iso_a4_210x297mm"));
    }

    #[test]
    fn absent_or_unknown_keys_leave_defaults() {
        let o = job_options_from_settings(&HashMap::new());
        assert_eq!(o.copies, None);
        assert!(o.duplex.is_none());
        assert!(o.color.is_none());
        assert!(o.media.is_none());

        // An unrecognised value for a known key is ignored, not guessed.
        let s = settings(&[("sides", "quad-sided"), ("n-copies", "0")]);
        let o = job_options_from_settings(&s);
        assert!(o.duplex.is_none());
        assert_eq!(o.copies, None); // 0 copies is not a valid override
    }

    #[test]
    fn copies_falls_back_between_key_names() {
        let o = job_options_from_settings(&settings(&[("copies", "5")]));
        assert_eq!(o.copies, Some(5));
    }
}
