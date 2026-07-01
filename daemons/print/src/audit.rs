//! The coarse print-audit (printing-plan.md §4: "a coarse audit entry records
//! 'app X printed <file> to <printer>'"). Printing is NOT gated - it stays a
//! user-driven action - but each job is recorded, and the print-as-egress angle
//! (§4.2: a network printer means the document leaves the machine) is made
//! visible by the audit KIND: a network destination records as a
//! [`AuditKind::NetworkCall`], a directly-attached one as [`AuditKind::Permission`].
//!
//! The structural tier stays content-free: the calling app id and the printer
//! name are coarse identifiers, the destination is local/network, the outcome is
//! a coarse label. The DOCUMENT - its name and its bytes - is never recorded
//! (the "<file>" in the plan's prose is for the user-facing activity view, not
//! the daemon-readable structural ledger).

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

use crate::model::{Destination, Printer};

/// Build the content-free `IngestRequest` for one print job.
///
/// `app_id` is the coarse id of the app that asked to print (carried as a label,
/// since the audit daemon attributes the connection actor to the portal, not the
/// app). The kind is [`AuditKind::NetworkCall`] for a network printer (the
/// document egresses) and [`AuditKind::Permission`] for a local one.
pub fn print_audit_event(app_id: &str, printer: &Printer, outcome: &str) -> IngestRequest {
    let kind = match printer.destination {
        Destination::Network => AuditKind::NetworkCall,
        Destination::Local => AuditKind::Permission,
    };
    IngestRequest {
        kind,
        structural: StructuralRecord {
            subject: format!("print.{}", printer.destination.as_key()),
            // Coarse identifiers only: who printed, and to which queue. No
            // document name and no bytes.
            node_types: vec![app_id.to_string(), printer.name.clone()],
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PrinterState;

    fn printer(name: &str, uri: &str) -> Printer {
        Printer::new(name, uri, None, None, None, PrinterState::Idle, true)
    }

    #[test]
    fn a_network_print_records_as_a_network_call() {
        let p = printer("Office", "ipp://printer.lan/ipp/print");
        let req = print_audit_event("org.example.editor", &p, "ok");
        assert_eq!(req.kind, AuditKind::NetworkCall, "network printing is egress");
        assert_eq!(req.structural.subject, "print.network");
        assert_eq!(req.structural.node_types, vec!["org.example.editor", "Office"]);
        assert_eq!(req.structural.outcome, "ok");
        req.validate().expect("within structural caps");
    }

    #[test]
    fn a_local_print_records_as_a_permission_event() {
        let p = printer("Desk", "usb://Brand/Model?serial=1");
        let req = print_audit_event("org.example.editor", &p, "ok");
        assert_eq!(req.kind, AuditKind::Permission);
        assert_eq!(req.structural.subject, "print.local");
    }

    #[test]
    fn the_document_never_reaches_the_structural_tier() {
        let p = printer("Office", "ipp://printer.lan/ipp/print");
        let req = print_audit_event("app", &p, "ok");
        let haystack = format!("{}{}", req.structural.subject, req.structural.node_types.join(","));
        // No document name / content is carried (only app + printer + dest).
        assert!(!haystack.contains(".pdf"));
        assert!(req.forensic.is_none());
    }
}
