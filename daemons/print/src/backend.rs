//! The print-backend boundary (printing-plan.md PRN-R1): the trait the portal
//! (PRN-R2) and the Settings panel (PRN-R4) call to enumerate printers, submit a
//! job, query the queue and cancel a job. The real implementation
//! ([`crate::cups::CupsBackend`], feature `cups`) speaks IPP to cupsd and is
//! verified on hardware; the [`MockBackend`] here lets the portal logic and the
//! callers be tested without a print server.

use async_trait::async_trait;

use crate::model::{Job, Printer};

/// A failure talking to the print system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    /// The print server was unavailable or returned an error.
    Backend(String),
    /// The named printer does not exist.
    NotFound(String),
    /// The submission was malformed (empty document, unknown printer name).
    Invalid(String),
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrintError::Backend(e) => write!(f, "print backend: {e}"),
            PrintError::NotFound(p) => write!(f, "no such printer: {p}"),
            PrintError::Invalid(e) => write!(f, "invalid print request: {e}"),
        }
    }
}

impl std::error::Error for PrintError {}

/// Sidedness for a job (IPP `sides`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duplex {
    /// Single-sided.
    OneSided,
    /// Double-sided, long-edge bind (portrait duplex).
    TwoSidedLongEdge,
    /// Double-sided, short-edge bind (landscape duplex).
    TwoSidedShortEdge,
}

impl Duplex {
    /// The IPP `sides` keyword.
    pub fn ipp_keyword(&self) -> &'static str {
        match self {
            Duplex::OneSided => "one-sided",
            Duplex::TwoSidedLongEdge => "two-sided-long-edge",
            Duplex::TwoSidedShortEdge => "two-sided-short-edge",
        }
    }
}

/// Colour mode for a job (IPP `print-color-mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Full colour.
    Color,
    /// Greyscale.
    Monochrome,
}

impl ColorMode {
    /// The IPP `print-color-mode` keyword.
    pub fn ipp_keyword(&self) -> &'static str {
        match self {
            ColorMode::Color => "color",
            ColorMode::Monochrome => "monochrome",
        }
    }
}

/// The job options the dialog (PRN-R3) sets. All optional: an unset option lets
/// the printer's default stand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobOptions {
    /// Number of copies (`copies`); `None` leaves the default (1).
    pub copies: Option<u32>,
    /// Duplex mode (`sides`).
    pub duplex: Option<Duplex>,
    /// Colour mode (`print-color-mode`).
    pub color: Option<ColorMode>,
    /// Media / paper size keyword (`media`, e.g. `iso_a4_210x297mm`).
    pub media: Option<String>,
}

/// One job submission: the target queue, the document bytes, and the options.
/// The document bytes are borrowed and never retained beyond the submit call;
/// the audit ([`crate::audit`]) records the printer and destination, never the
/// document.
pub struct PrintSubmission<'a> {
    /// The target queue name.
    pub printer: &'a str,
    /// The document bytes.
    pub document: &'a [u8],
    /// The job / document title (`job-name`), if any.
    pub title: Option<&'a str>,
    /// The document MIME type (`document-format`, e.g. `application/pdf`); `None`
    /// lets CUPS auto-detect.
    pub mime: Option<&'a str>,
    /// The job options.
    pub options: JobOptions,
}

/// The print system the portal and Settings call.
#[async_trait]
pub trait PrintBackend: Send + Sync {
    /// Enumerate configured printer queues.
    async fn printers(&self) -> Result<Vec<Printer>, PrintError>;
    /// Query the queue: all jobs, or just one printer's when `printer` is set.
    async fn jobs(&self, printer: Option<&str>) -> Result<Vec<Job>, PrintError>;
    /// Submit a job; returns the assigned `job-id`.
    async fn submit(&self, submission: &PrintSubmission<'_>) -> Result<i32, PrintError>;
    /// Cancel a job in a queue.
    async fn cancel_job(&self, printer: &str, job_id: i32) -> Result<(), PrintError>;
}

/// An in-memory [`PrintBackend`] for tests: a fixed printer set, an incrementing
/// job id, and a record of what was submitted (so a caller can assert it printed
/// to the right queue without a cupsd).
#[cfg(any(test, feature = "mock"))]
pub struct MockBackend {
    printers: Vec<Printer>,
    state: std::sync::Mutex<MockState>,
}

#[cfg(any(test, feature = "mock"))]
#[derive(Default)]
struct MockState {
    next_id: i32,
    jobs: Vec<Job>,
    /// (printer, copies, title) of each accepted submission, for assertions.
    submitted: Vec<(String, u32, Option<String>)>,
}

#[cfg(any(test, feature = "mock"))]
impl MockBackend {
    /// A mock serving the given printers.
    pub fn new(printers: Vec<Printer>) -> Self {
        Self {
            printers,
            state: std::sync::Mutex::new(MockState {
                next_id: 1,
                ..Default::default()
            }),
        }
    }

    /// The submissions accepted so far: (printer, copies, title).
    pub fn submissions(&self) -> Vec<(String, u32, Option<String>)> {
        self.state.lock().unwrap().submitted.clone()
    }
}

#[cfg(any(test, feature = "mock"))]
#[async_trait]
impl PrintBackend for MockBackend {
    async fn printers(&self) -> Result<Vec<Printer>, PrintError> {
        Ok(self.printers.clone())
    }

    async fn jobs(&self, printer: Option<&str>) -> Result<Vec<Job>, PrintError> {
        let jobs = self.state.lock().unwrap().jobs.clone();
        Ok(match printer {
            Some(p) => jobs.into_iter().filter(|j| j.printer == p).collect(),
            None => jobs,
        })
    }

    async fn submit(&self, submission: &PrintSubmission<'_>) -> Result<i32, PrintError> {
        if submission.document.is_empty() {
            return Err(PrintError::Invalid("empty document".into()));
        }
        if !self.printers.iter().any(|p| p.name == submission.printer) {
            return Err(PrintError::NotFound(submission.printer.to_string()));
        }
        let mut st = self.state.lock().unwrap();
        let id = st.next_id;
        st.next_id += 1;
        st.jobs.push(Job {
            id,
            printer: submission.printer.to_string(),
            name: submission.title.map(str::to_string),
            user: None,
            state: crate::model::JobState::Pending,
        });
        st.submitted.push((
            submission.printer.to_string(),
            submission.options.copies.unwrap_or(1),
            submission.title.map(str::to_string),
        ));
        Ok(id)
    }

    async fn cancel_job(&self, printer: &str, job_id: i32) -> Result<(), PrintError> {
        let mut st = self.state.lock().unwrap();
        let before = st.jobs.len();
        st.jobs.retain(|j| !(j.printer == printer && j.id == job_id));
        if st.jobs.len() == before {
            return Err(PrintError::NotFound(format!("{printer}#{job_id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Printer, PrinterState};

    fn printer(name: &str, uri: &str) -> Printer {
        Printer::new(name, uri, None, None, None, PrinterState::Idle, true)
    }

    #[tokio::test]
    async fn submit_records_the_job_and_assigns_ids() {
        let backend = MockBackend::new(vec![printer("Office", "ipp://printer.lan/ipp/print")]);
        let sub = PrintSubmission {
            printer: "Office",
            document: b"%PDF-1.7 ...",
            title: Some("report.pdf"),
            mime: Some("application/pdf"),
            options: JobOptions {
                copies: Some(2),
                ..Default::default()
            },
        };
        let id = backend.submit(&sub).await.expect("submit");
        assert_eq!(id, 1);
        let jobs = backend.jobs(Some("Office")).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name.as_deref(), Some("report.pdf"));
        assert_eq!(backend.submissions(), vec![("Office".to_string(), 2, Some("report.pdf".to_string()))]);
    }

    #[tokio::test]
    async fn submit_rejects_empty_document_and_unknown_printer() {
        let backend = MockBackend::new(vec![printer("Office", "usb://x/y")]);
        let empty = PrintSubmission {
            printer: "Office",
            document: b"",
            title: None,
            mime: None,
            options: JobOptions::default(),
        };
        assert_eq!(backend.submit(&empty).await, Err(PrintError::Invalid("empty document".into())));
        let unknown = PrintSubmission {
            printer: "Ghost",
            document: b"x",
            title: None,
            mime: None,
            options: JobOptions::default(),
        };
        assert_eq!(backend.submit(&unknown).await, Err(PrintError::NotFound("Ghost".into())));
    }

    #[tokio::test]
    async fn cancel_removes_a_job_or_reports_not_found() {
        let backend = MockBackend::new(vec![printer("Office", "usb://x/y")]);
        let sub = PrintSubmission {
            printer: "Office",
            document: b"x",
            title: None,
            mime: None,
            options: JobOptions::default(),
        };
        let id = backend.submit(&sub).await.unwrap();
        backend.cancel_job("Office", id).await.expect("cancel");
        assert!(backend.jobs(None).await.unwrap().is_empty());
        assert!(backend.cancel_job("Office", id).await.is_err());
    }

    #[test]
    fn ipp_keywords_are_stable() {
        assert_eq!(Duplex::TwoSidedLongEdge.ipp_keyword(), "two-sided-long-edge");
        assert_eq!(ColorMode::Monochrome.ipp_keyword(), "monochrome");
    }
}
