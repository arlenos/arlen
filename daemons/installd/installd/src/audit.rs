//! Content-free audit of install/uninstall actions (GAP-2).
//!
//! Installing or removing software is a root-privileged system change worth
//! recording in the audit ledger. Like every structural record it carries only
//! coarse identifiers: the action (`install`/`uninstall`), the source
//! (`lunpkg`/`flatpak`), the affected app's coarse reverse-DNS id when known,
//! and the outcome. A local-file install's only input is a filesystem path,
//! which is NOT carried — the ledger says installd installed a package and how
//! it went, never the path, package contents or signing material.
//!
//! The audited ACTOR is the install daemon itself (kernel-attested at the
//! ingest socket via SO_PEERCRED → `installd`); the affected app id, when
//! present, travels as a coarse `node_type`.

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

/// Build the content-free `IngestRequest` for one install/uninstall action.
///
/// `action` is `install` or `uninstall`, `source` is `lunpkg` or `flatpak`,
/// `subject_id` is the affected app's coarse reverse-DNS id when known (absent
/// for a local-file install, whose only input is a path that must not be
/// carried), and `outcome` is `ok` or `failed`. Records as
/// [`AuditKind::AppAction`].
pub fn install_action_event(
    action: &str,
    source: &str,
    subject_id: Option<&str>,
    outcome: &str,
) -> IngestRequest {
    let mut node_types = vec![source.to_string()];
    if let Some(id) = subject_id {
        node_types.push(id.to_string());
    }
    IngestRequest {
        kind: AuditKind::AppAction,
        structural: StructuralRecord {
            subject: format!("package.{action}"),
            node_types,
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uninstall_records_source_and_app_id() {
        let req = install_action_event("uninstall", "flatpak", Some("com.example.App"), "ok");
        assert_eq!(req.kind, AuditKind::AppAction);
        assert_eq!(req.structural.subject, "package.uninstall");
        assert_eq!(req.structural.node_types, vec!["flatpak", "com.example.App"]);
        assert_eq!(req.structural.outcome, "ok");
        req.validate().expect("within structural caps");
    }

    #[test]
    fn a_local_file_install_carries_no_path() {
        // A .lunpkg install's only D-Bus input is a filesystem path. With no
        // coarse app id at the worker, the record carries only the source and
        // outcome — never the path.
        let req = install_action_event("install", "lunpkg", None, "ok");
        assert_eq!(req.structural.subject, "package.install");
        assert_eq!(req.structural.node_types, vec!["lunpkg"]);
        assert!(req.forensic.is_none());
        req.validate().expect("within structural caps");
    }
}
