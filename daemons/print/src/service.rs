//! The print service layer (printing-plan.md PRN-R1): the reusable
//! submit-and-audit + query surface over a [`PrintBackend`] that both the portal
//! Print backend (PRN-R2) and the Settings Printers panel (PRN-R4) call.
//!
//! Printing is NOT gated - it is a user-driven action (printing-plan.md §4), so
//! the audit is a RECORD written AFTER the job is accepted, best-effort: a print
//! is never blocked or failed because the audit ledger is momentarily
//! unavailable (unlike the AI fail-closed-before-act path). The record is still
//! the security posture (print-as-egress visibility), so an audit failure is
//! logged, not swallowed silently.
//!
//! What the service does NOT do: choose the printer. The portal-isolation
//! property (§4.1, "an app cannot print silently") means the USER picks the
//! printer in the dialog (PRN-R3); the service receives an already-chosen
//! printer name and submits to it. It never auto-selects a default on the app's
//! behalf.

use std::sync::Arc;

use audit_proto::sink::AuditSink;

use crate::audit::print_audit_event;
use crate::backend::{PrintBackend, PrintError, PrintSubmission};
use crate::model::{Job, Printer};

/// The print service: a backend plus the audit sink jobs are recorded to.
pub struct PrintService<B: PrintBackend> {
    backend: B,
    audit: Arc<dyn AuditSink>,
}

impl<B: PrintBackend> PrintService<B> {
    /// Build a service over a backend and an audit sink.
    pub fn new(backend: B, audit: Arc<dyn AuditSink>) -> Self {
        Self { backend, audit }
    }

    /// Enumerate configured printers.
    pub async fn printers(&self) -> Result<Vec<Printer>, PrintError> {
        self.backend.printers().await
    }

    /// Query the queue (all, or one printer's).
    pub async fn jobs(&self, printer: Option<&str>) -> Result<Vec<Job>, PrintError> {
        self.backend.jobs(printer).await
    }

    /// Cancel a job.
    pub async fn cancel_job(&self, printer: &str, job_id: i32) -> Result<(), PrintError> {
        self.backend.cancel_job(printer, job_id).await
    }

    /// Submit a print job to the user-chosen `printer` on behalf of `app_id`,
    /// then record it.
    ///
    /// The submission is attempted first; only an ACCEPTED job is recorded (so
    /// the ledger reflects what actually printed). The audit is best-effort - a
    /// sink failure is logged but does not fail the print, because printing is a
    /// user-driven action that must not be blocked by a logging hiccup. The
    /// document bytes are never audited; only the app id, the printer and the
    /// local/network destination are.
    pub async fn submit(
        &self,
        app_id: &str,
        submission: &PrintSubmission<'_>,
    ) -> Result<i32, PrintError> {
        // Resolve the chosen printer first so the audit can carry its
        // destination (local vs network) even before the job id is known.
        let printer = self
            .backend
            .printers()
            .await?
            .into_iter()
            .find(|p| p.name == submission.printer)
            .ok_or_else(|| PrintError::NotFound(submission.printer.to_string()))?;

        let result = self.backend.submit(submission).await;
        let outcome = match &result {
            Ok(_) => "ok",
            Err(_) => "error",
        };
        // Record the attempt (best-effort): both the accepted job and a rejected
        // one are worth a ledger line, with no document content either way.
        let event = print_audit_event(app_id, &printer, outcome);
        if let Err(e) = self.audit.submit(event).await {
            tracing::warn!("print audit record failed (print still proceeded): {e}");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{JobOptions, MockBackend};
    use crate::model::{Printer, PrinterState};
    use audit_proto::sink::MockAuditSink;
    use audit_proto::AuditKind;

    fn printer(name: &str, uri: &str) -> Printer {
        Printer::new(name, uri, None, None, None, PrinterState::Idle, true)
    }

    fn service(printers: Vec<Printer>) -> (PrintService<MockBackend>, Arc<MockAuditSink>) {
        let audit = Arc::new(MockAuditSink::accepting());
        let svc = PrintService::new(MockBackend::new(printers), audit.clone());
        (svc, audit)
    }

    #[tokio::test]
    async fn a_network_print_submits_and_records_a_network_call() {
        let (svc, audit) = service(vec![printer("Office", "ipp://printer.lan/ipp/print")]);
        let sub = PrintSubmission {
            printer: "Office",
            document: b"%PDF...",
            title: Some("report"),
            mime: Some("application/pdf"),
            options: JobOptions::default(),
        };
        let id = svc.submit("org.example.app", &sub).await.expect("submit");
        assert_eq!(id, 1);
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1, "one audit record for the print");
        assert_eq!(recorded[0].kind, AuditKind::NetworkCall, "network printer = egress");
        assert_eq!(recorded[0].structural.subject, "print.network");
        assert_eq!(recorded[0].structural.outcome, "ok");
    }

    #[tokio::test]
    async fn submitting_to_an_unknown_printer_is_not_found_and_not_recorded() {
        let (svc, audit) = service(vec![printer("Office", "usb://x/y")]);
        let sub = PrintSubmission {
            printer: "Ghost",
            document: b"x",
            title: None,
            mime: None,
            options: JobOptions::default(),
        };
        assert_eq!(svc.submit("app", &sub).await, Err(PrintError::NotFound("Ghost".into())));
        // No printer resolved -> nothing recorded (we record real attempts only).
        assert!(audit.recorded().await.is_empty());
    }

    #[tokio::test]
    async fn a_rejected_submission_is_recorded_as_an_error() {
        let (svc, audit) = service(vec![printer("Office", "usb://x/y")]);
        let empty = PrintSubmission {
            printer: "Office",
            document: b"", // the mock rejects an empty document
            title: None,
            mime: None,
            options: JobOptions::default(),
        };
        assert!(svc.submit("app", &empty).await.is_err());
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].structural.outcome, "error");
        assert_eq!(recorded[0].kind, AuditKind::Permission, "local printer = permission event");
    }

    #[tokio::test]
    async fn printers_and_jobs_pass_through_to_the_backend() {
        let (svc, _audit) = service(vec![printer("Office", "usb://x/y")]);
        assert_eq!(svc.printers().await.unwrap().len(), 1);
        assert!(svc.jobs(None).await.unwrap().is_empty());
    }
}
