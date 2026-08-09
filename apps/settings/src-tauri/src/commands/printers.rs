//! Printers panel backend: the read and job-cancel operations over the local
//! CUPS server via the shared `arlen-print` IPP backend.
//!
//! These are the unprivileged half of the panel: list the configured printers,
//! read the default, list the active queue, and cancel a job (a user may cancel
//! their own jobs). Adding, removing, or reconfiguring a printer is a CUPS admin
//! operation (needs lpadmin/polkit) handled by the deliberate admin extension,
//! not here. Each command talks directly to the local CUPS server; a down or
//! unreachable server surfaces as an error string the panel shows.

use arlen_print::{CupsBackend, Job, JobOptions, PrintBackend, PrintSubmission};
use serde::Serialize;

/// The panel's printer rows come from `arlen-printers`, the shared view: the
/// shell's print dialog lists the same rows, and a second copy of the mapping
/// here is how the two drift apart.
pub use arlen_printers::PrinterView as PrinterDto;

/// A print job as the panel lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDto {
    id: i32,
    printer: String,
    name: Option<String>,
    user: Option<String>,
    state: String,
}

impl From<Job> for JobDto {
    fn from(j: Job) -> Self {
        Self {
            id: j.id,
            printer: j.printer,
            name: j.name,
            user: j.user,
            state: j.state.as_key().to_string(),
        }
    }
}

/// The configured printer queues from the local CUPS server.
#[tauri::command]
pub async fn printers_list() -> Result<Vec<PrinterDto>, String> {
    arlen_printers::list().await
}

/// The default printer, if one is set.
#[tauri::command]
pub async fn printers_default() -> Result<Option<PrinterDto>, String> {
    arlen_printers::default_printer().await
}

/// Make a printer this user's default.
///
/// The USER default, in `lpoptions`, not the machine's: changing what every
/// account prints to needs printer-admin rights over IPP, and a settings toggle
/// that quietly did that to other people's sessions would be the wrong scope for
/// the word "default". Per-user is also what `lpoptions -d` means, so anything
/// else reading CUPS for this account agrees with the page.
#[tauri::command]
pub async fn printers_set_default(name: String) -> Result<(), String> {
    arlen_print::user_default::set_default(&name).map_err(|e| e.to_string())
}

/// The paper size, duplex and colour this user's jobs get on `printer` unless a
/// dialog overrides them.
///
/// Per-user, in `lpoptions`, for the same reason as the default: a machine-wide
/// printer default is an admin operation. The keys are IPP attribute names
/// because that is what CUPS reads back, so a value set here is the value the
/// print dialog and `lp` both see.
#[tauri::command]
pub async fn printers_set_options(name: String, options: PrinterOptionsDto) -> Result<(), String> {
    let pairs = vec![
        ("media".to_string(), options.paper_ipp().to_string()),
        ("sides".to_string(), options.duplex_ipp().to_string()),
        ("print-color-mode".to_string(), options.color_ipp().to_string()),
    ];
    arlen_print::user_default::set_dest_options(&name, &pairs).map_err(|e| e.to_string())
}

/// What the Printers panel sends for one printer's saved options. The strings
/// are the frontend's own vocabulary; the IPP keywords they map to are below, so
/// an unknown value falls back rather than being written through.
#[derive(Debug, serde::Deserialize)]
pub struct PrinterOptionsDto {
    /// `one-sided` | `two-sided-long` | `two-sided-short`.
    pub duplex: String,
    /// `color` | `mono`.
    pub color: String,
    /// `a4` | `letter` | `legal`.
    pub paper: String,
}

impl PrinterOptionsDto {
    fn duplex_ipp(&self) -> &'static str {
        match self.duplex.as_str() {
            "two-sided-long" => "two-sided-long-edge",
            "two-sided-short" => "two-sided-short-edge",
            _ => "one-sided",
        }
    }

    fn color_ipp(&self) -> &'static str {
        match self.color.as_str() {
            "mono" => "monochrome",
            _ => "color",
        }
    }

    /// PWG media names, which is what CUPS stores.
    fn paper_ipp(&self) -> &'static str {
        match self.paper.as_str() {
            "letter" => "na_letter_8.5x11in",
            "legal" => "na_legal_8.5x14in",
            _ => "iso_a4_210x297mm",
        }
    }
}

/// The active print queue across all printers (pending, held, and processing
/// jobs).
#[tauri::command]
pub async fn print_queue() -> Result<Vec<JobDto>, String> {
    CupsBackend::default()
        .jobs(None)
        .await
        .map(|js| js.into_iter().map(JobDto::from).collect())
        .map_err(|e| e.to_string())
}

/// Cancel a print job in `printer`'s queue by its IPP job id.
#[tauri::command]
pub async fn print_job_cancel(printer: String, job_id: i32) -> Result<(), String> {
    CupsBackend::default()
        .cancel_job(&printer, job_id)
        .await
        .map_err(|e| e.to_string())
}

/// Queue a test page on `printer` to confirm it prints. A plain-text document
/// (CUPS auto-detects `text/plain`) submitted as a normal user job, so no admin
/// rights are needed. Returns the IPP job id the panel can report.
#[tauri::command]
pub async fn printers_test_page(printer: String) -> Result<i32, String> {
    const TEST_DOC: &[u8] = b"Arlen printer test page\n\nIf you can read this, printing works.\n";
    let submission = PrintSubmission {
        printer: &printer,
        document: TEST_DOC,
        title: Some("Arlen Test Page"),
        mime: Some("text/plain"),
        options: JobOptions::default(),
    };
    CupsBackend::default()
        .submit(&submission)
        .await
        .map_err(|e| e.to_string())
}
