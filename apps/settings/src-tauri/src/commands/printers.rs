//! Printers panel backend: the read and job-cancel operations over the local
//! CUPS server via the shared `arlen-print` IPP backend.
//!
//! These are the unprivileged half of the panel: list the configured printers,
//! read the default, list the active queue, and cancel a job (a user may cancel
//! their own jobs). Adding, removing, or reconfiguring a printer is a CUPS admin
//! operation (needs lpadmin/polkit) handled by the deliberate admin extension,
//! not here. Each command talks directly to the local CUPS server; a down or
//! unreachable server surfaces as an error string the panel shows.

use arlen_print::{CupsBackend, Job, PrintBackend, Printer};
use serde::Serialize;

/// A printer as the panel lists it. The `arlen-print` `Printer` is not
/// serializable and the panel wants stable lowercase state keys, so this is the
/// wire shape (camelCase for the frontend).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterDto {
    name: String,
    uri: String,
    info: Option<String>,
    location: Option<String>,
    make_model: Option<String>,
    state: String,
    accepting_jobs: bool,
    destination: String,
}

impl From<Printer> for PrinterDto {
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
    CupsBackend::default()
        .printers()
        .await
        .map(|ps| ps.into_iter().map(PrinterDto::from).collect())
        .map_err(|e| e.to_string())
}

/// The default printer, if one is set.
#[tauri::command]
pub async fn printers_default() -> Result<Option<PrinterDto>, String> {
    CupsBackend::default()
        .default_printer()
        .await
        .map(|p| p.map(PrinterDto::from))
        .map_err(|e| e.to_string())
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
