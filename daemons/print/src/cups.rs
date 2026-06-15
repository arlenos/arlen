//! The real CUPS/IPP backend (printing-plan.md PRN-R1), feature-gated behind
//! `cups`. It speaks IPP to the local cupsd via the pure-Rust `ipp` crate (no
//! libcups linking) over plain HTTP to `localhost:631` (a loopback queue, so no
//! TLS). The live `.send()` round-trips are verified against a running cupsd on
//! hardware (the PWR-R2 metal pattern); the bug-prone part - turning an IPP
//! attribute group into a typed [`Printer`] / [`Job`] - is extracted into pure
//! helpers tested against synthetic groups here.
//!
//! Driverless-first (printing-plan.md §1): `CUPS-Get-Printers` returns every
//! configured queue, including the zero-config IPP Everywhere queues cupsd
//! auto-creates from DNS-SD, so Avahi discovery needs no separate browse - the
//! queue simply appears. (A manual mDNS browse for not-yet-added printers is a
//! later add.)

use async_trait::async_trait;
use ipp::prelude::*;
use ipp::attribute::IppAttributeGroup;
use ipp::model::DelimiterTag;
use ipp::value::IppValue;

use crate::backend::{PrintBackend, PrintError, PrintSubmission};
use crate::model::{Job, JobState, Printer, PrinterState};

/// The default cupsd endpoint: the loopback IPP port.
pub const DEFAULT_CUPS_URI: &str = "http://localhost:631/";

/// A [`PrintBackend`] talking IPP to cupsd.
pub struct CupsBackend {
    base: String,
}

impl Default for CupsBackend {
    fn default() -> Self {
        Self::new(DEFAULT_CUPS_URI)
    }
}

impl CupsBackend {
    /// A backend against an explicit cupsd base URI (e.g. `http://localhost:631/`).
    pub fn new(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }

    /// Parse the base URI, mapping a malformed value to a backend error.
    fn base_uri(&self) -> Result<Uri, PrintError> {
        self.base
            .parse::<Uri>()
            .map_err(|e| PrintError::Backend(format!("bad cups uri: {e}")))
    }

    /// The IPP URI of one named queue under this cupsd.
    fn printer_uri(&self, name: &str) -> Result<Uri, PrintError> {
        format!("{}printers/{name}", self.base)
            .parse::<Uri>()
            .map_err(|e| PrintError::Backend(format!("bad printer uri: {e}")))
    }
}

/// The string value of an attribute (keyword / name / text / uri), via Display.
fn attr_text(group: &IppAttributeGroup, name: &str) -> Option<String> {
    group
        .attributes()
        .values()
        .find(|a| a.name().as_str() == name)
        .map(|a| a.value().to_string())
}

/// The integer value of an attribute (`integer` or `enum`).
fn attr_int(group: &IppAttributeGroup, name: &str) -> Option<i32> {
    let v = group
        .attributes()
        .values()
        .find(|a| a.name().as_str() == name)?
        .value();
    value_int(v)
}

/// The boolean value of an attribute.
fn attr_bool(group: &IppAttributeGroup, name: &str) -> Option<bool> {
    let v = group
        .attributes()
        .values()
        .find(|a| a.name().as_str() == name)?
        .value();
    v.as_boolean().copied()
}

/// An IPP integer-or-enum value as i32.
fn value_int(v: &IppValue) -> Option<i32> {
    v.as_integer().or_else(|| v.as_enum()).copied()
}

/// Build a [`Printer`] from a `printer-attributes` group. Returns `None` if the
/// group has no `printer-name` (not a printer entry).
pub(crate) fn printer_from_group(group: &IppAttributeGroup) -> Option<Printer> {
    let name = attr_text(group, "printer-name")?;
    // CUPS reports the backend device as `device-uri`; fall back to the
    // advertised `printer-uri-supported` if the device uri is hidden.
    let uri = attr_text(group, "device-uri")
        .or_else(|| attr_text(group, "printer-uri-supported"))
        .unwrap_or_default();
    let state = attr_int(group, "printer-state")
        .map(PrinterState::from_ipp)
        .unwrap_or(PrinterState::Unknown(0));
    let accepting = attr_bool(group, "printer-is-accepting-jobs").unwrap_or(false);
    Some(Printer::new(
        name,
        uri,
        attr_text(group, "printer-info"),
        attr_text(group, "printer-location"),
        attr_text(group, "printer-make-and-model"),
        state,
        accepting,
    ))
}

/// Build a [`Job`] from a `job-attributes` group, attributing it to `printer`.
/// Returns `None` if the group has no `job-id`.
pub(crate) fn job_from_group(group: &IppAttributeGroup, printer: &str) -> Option<Job> {
    let id = attr_int(group, "job-id")?;
    let state = attr_int(group, "job-state")
        .map(JobState::from_ipp)
        .unwrap_or(JobState::Unknown(0));
    Some(Job {
        id,
        printer: printer.to_string(),
        name: attr_text(group, "job-name"),
        user: attr_text(group, "job-originating-user-name"),
        state,
    })
}

#[async_trait]
impl PrintBackend for CupsBackend {
    async fn printers(&self) -> Result<Vec<Printer>, PrintError> {
        let uri = self.base_uri()?;
        // Bind the builder: get_printers borrows it, and the returned operation
        // must outlive the temporary (Rust 2024 impl-Trait capture).
        let cups = IppOperationBuilder::cups();
        let op = cups.get_printers();
        let client = AsyncIppClient::new(uri);
        let resp = client
            .send(op)
            .await
            .map_err(|e| PrintError::Backend(e.to_string()))?;
        if !resp.header().status_code().is_success() {
            return Err(PrintError::Backend(format!(
                "cups-get-printers: {}",
                resp.header().status_code()
            )));
        }
        // Materialise into owned printers before `resp` (which the group iterator
        // borrows) goes out of scope.
        let printers: Vec<Printer> = resp
            .attributes()
            .groups_of(DelimiterTag::PrinterAttributes)
            .filter_map(printer_from_group)
            .collect();
        Ok(printers)
    }

    async fn jobs(&self, printer: Option<&str>) -> Result<Vec<Job>, PrintError> {
        // CUPS Get-Jobs is per-printer; with no printer, query each queue.
        let names: Vec<String> = match printer {
            Some(p) => vec![p.to_string()],
            None => self.printers().await?.into_iter().map(|p| p.name).collect(),
        };
        let mut out = Vec::new();
        for name in names {
            let uri = self.printer_uri(&name)?;
            let op = IppOperationBuilder::get_jobs(uri.clone())
                .build()
                .map_err(|e| PrintError::Backend(e.to_string()))?;
            let client = AsyncIppClient::new(uri);
            let resp = client
                .send(op)
                .await
                .map_err(|e| PrintError::Backend(e.to_string()))?;
            if !resp.header().status_code().is_success() {
                return Err(PrintError::Backend(format!(
                    "get-jobs: {}",
                    resp.header().status_code()
                )));
            }
            let jobs: Vec<Job> = resp
                .attributes()
                .groups_of(DelimiterTag::JobAttributes)
                .filter_map(|g| job_from_group(g, &name))
                .collect();
            out.extend(jobs);
        }
        Ok(out)
    }

    async fn submit(&self, submission: &PrintSubmission<'_>) -> Result<i32, PrintError> {
        if submission.document.is_empty() {
            return Err(PrintError::Invalid("empty document".into()));
        }
        let uri = self.printer_uri(submission.printer)?;
        let payload =
            IppPayload::new_async(futures_util::io::Cursor::new(submission.document.to_vec()));
        let mut builder = IppOperationBuilder::print_job(uri.clone(), payload);
        if let Some(title) = submission.title {
            builder = builder.job_title(title);
        }
        if let Some(mime) = submission.mime {
            builder = builder.document_format(mime);
        }
        for attr in job_option_attributes(&submission.options) {
            builder = builder.attribute(attr);
        }
        let op = builder
            .build()
            .map_err(|e| PrintError::Backend(e.to_string()))?;
        let client = AsyncIppClient::new(uri);
        let resp = client
            .send(op)
            .await
            .map_err(|e| PrintError::Backend(e.to_string()))?;
        if !resp.header().status_code().is_success() {
            return Err(PrintError::Backend(format!(
                "print-job: {}",
                resp.header().status_code()
            )));
        }
        let job_id = resp
            .attributes()
            .groups_of(DelimiterTag::JobAttributes)
            .find_map(|g| attr_int(g, "job-id"));
        job_id.ok_or_else(|| PrintError::Backend("print-job returned no job-id".into()))
    }

    async fn cancel_job(&self, printer: &str, job_id: i32) -> Result<(), PrintError> {
        let uri = self.printer_uri(printer)?;
        let op = IppOperationBuilder::cancel_job(uri.clone(), job_id)
            .build()
            .map_err(|e| PrintError::Backend(e.to_string()))?;
        let client = AsyncIppClient::new(uri);
        let resp = client
            .send(op)
            .await
            .map_err(|e| PrintError::Backend(e.to_string()))?;
        if !resp.header().status_code().is_success() {
            return Err(PrintError::Backend(format!(
                "cancel-job: {}",
                resp.header().status_code()
            )));
        }
        Ok(())
    }
}

/// Turn the typed [`JobOptions`](crate::backend::JobOptions) into IPP job
/// attributes for a Print-Job request. Only set options produce an attribute;
/// an unset option lets the printer default stand.
fn job_option_attributes(opts: &crate::backend::JobOptions) -> Vec<IppAttribute> {
    let mut attrs = Vec::new();
    // Each attribute name and keyword is constructed fallibly (the IPP string
    // types are length-bounded); a value that does not fit is skipped rather
    // than failing the whole job (only the user-supplied `media` could ever
    // exceed, and dropping it lets the printer default stand).
    let mut push = |name: &str, value: IppValue| {
        if let Ok(attr) = IppAttribute::with_name(name, value) {
            attrs.push(attr);
        }
    };
    if let Some(copies) = opts.copies {
        push("copies", IppValue::Integer(copies as i32));
    }
    if let Some(duplex) = opts.duplex {
        if let Ok(kw) = duplex.ipp_keyword().parse() {
            push("sides", IppValue::Keyword(kw));
        }
    }
    if let Some(color) = opts.color {
        if let Ok(kw) = color.ipp_keyword().parse() {
            push("print-color-mode", IppValue::Keyword(kw));
        }
    }
    if let Some(media) = &opts.media {
        if let Ok(kw) = media.parse() {
            push("media", IppValue::Keyword(kw));
        }
    }
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{ColorMode, Duplex, JobOptions};
    use crate::model::Destination;

    fn name(s: &str) -> ipp::value::IppName {
        s.parse().expect("valid ipp name")
    }
    fn text(s: &str) -> ipp::value::IppTextValue {
        s.try_into().expect("valid ipp text")
    }
    fn uri(s: &str) -> ipp::value::IppString {
        s.parse().expect("valid ipp string")
    }

    fn group(tag: DelimiterTag, attrs: Vec<(&str, IppValue)>) -> IppAttributeGroup {
        let mut g = IppAttributeGroup::new(tag);
        for (n, value) in attrs {
            let attr = IppAttribute::with_name(n, value).expect("valid attr name");
            g.attributes_mut().insert(name(n), attr);
        }
        g
    }

    #[test]
    fn printer_from_group_extracts_a_network_printer() {
        let g = group(
            DelimiterTag::PrinterAttributes,
            vec![
                ("printer-name", IppValue::NameWithoutLanguage(name("Office"))),
                ("device-uri", IppValue::Uri(uri("ipp://printer.lan/ipp/print"))),
                ("printer-info", IppValue::TextWithoutLanguage(text("Front desk"))),
                ("printer-state", IppValue::Enum(3)),
                ("printer-is-accepting-jobs", IppValue::Boolean(true)),
                ("printer-make-and-model", IppValue::TextWithoutLanguage(text("Brand X"))),
            ],
        );
        let p = printer_from_group(&g).expect("a printer");
        assert_eq!(p.name, "Office");
        assert_eq!(p.uri, "ipp://printer.lan/ipp/print");
        assert_eq!(p.state, PrinterState::Idle);
        assert!(p.accepting_jobs);
        assert_eq!(p.destination, Destination::Network);
        assert_eq!(p.make_model.as_deref(), Some("Brand X"));
    }

    #[test]
    fn printer_from_group_needs_a_name() {
        let g = group(
            DelimiterTag::PrinterAttributes,
            vec![("printer-state", IppValue::Enum(3))],
        );
        assert!(printer_from_group(&g).is_none());
    }

    #[test]
    fn job_from_group_extracts_and_attributes_to_the_queue() {
        let g = group(
            DelimiterTag::JobAttributes,
            vec![
                ("job-id", IppValue::Integer(42)),
                ("job-state", IppValue::Enum(5)),
                ("job-name", IppValue::NameWithoutLanguage(name("report"))),
                ("job-originating-user-name", IppValue::NameWithoutLanguage(name("alice"))),
            ],
        );
        let j = job_from_group(&g, "Office").expect("a job");
        assert_eq!(j.id, 42);
        assert_eq!(j.printer, "Office");
        assert_eq!(j.state, JobState::Processing);
        assert_eq!(j.name.as_deref(), Some("report"));
        assert_eq!(j.user.as_deref(), Some("alice"));
    }

    #[test]
    fn job_from_group_needs_an_id() {
        let g = group(DelimiterTag::JobAttributes, vec![("job-state", IppValue::Enum(3))]);
        assert!(job_from_group(&g, "Office").is_none());
    }

    #[test]
    fn job_options_become_only_the_set_ipp_attributes() {
        let none = job_option_attributes(&JobOptions::default());
        assert!(none.is_empty(), "no options -> no attributes (printer defaults stand)");
        let opts = JobOptions {
            copies: Some(3),
            duplex: Some(Duplex::TwoSidedLongEdge),
            color: Some(ColorMode::Monochrome),
            media: Some("iso_a4_210x297mm".into()),
        };
        let attrs = job_option_attributes(&opts);
        let names: Vec<&str> = attrs.iter().map(|a| a.name().as_str()).collect();
        assert!(names.contains(&"copies"));
        assert!(names.contains(&"sides"));
        assert!(names.contains(&"print-color-mode"));
        assert!(names.contains(&"media"));
        // copies is an integer; the rest are keywords.
        let copies = attrs.iter().find(|a| a.name().as_str() == "copies").unwrap();
        assert_eq!(value_int(copies.value()), Some(3));
    }
}
