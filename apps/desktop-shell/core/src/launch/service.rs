//! Serving a launch request: who asked, whether they may, and what to record.
//!
//! The socket's decision layer, without the socket. The host accepts the
//! connection, reads the peer credential and does the I/O; this decides.
//!
//! **The gate is about attribution, not authority.** A same-uid process can
//! already `exec` anything it likes, so a launch request hands it nothing it
//! lacks - naming an application does not lift its privileges, and the confined
//! path gives the started program *less* than an unconfined spawn would. What
//! the socket adds is that the system knows who asked, which is why the audit
//! line matters more here than a program allowlist would.
//!
//! So the two request shapes are treated differently, on purpose:
//!
//! - [`LaunchRequest::Open`] is served to any caller, named or not. It grants
//!   nothing, and refusing an unrecognised binary would only cost someone the
//!   ability to open a document.
//! - [`LaunchRequest::App`] needs a named caller. It is the shape that makes a
//!   claim about the user's setup - wanting a *specific* application rather than
//!   whatever opens a document - and "app X caused app Y to start" is only a
//!   sentence if X has a name.

use super::mimeapps::MimeApps;
use super::plan::Launch;
use super::request::{resolve, Entry, LaunchError};
use arlen_launch_contract::{LaunchOutcome, LaunchRequest};

/// Who asked, as the socket attested them - never as they claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// The peer resolved to a known application.
    Named(String),
    /// The peer is same-uid but its binary resolves to nothing the identity
    /// resolver recognises. Not a failure by itself: most of what a session runs
    /// is not a packaged application.
    Unnamed,
}

impl Caller {
    /// The name for the ledger.
    ///
    /// An unresolved caller is written out as unresolved rather than omitted. A
    /// served `Open` still causes a program to start - the target and its type
    /// select a handler - so an audit line with no caller field would read as
    /// though nothing caused it. This states what could not be checked, the way
    /// a gate does.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Named(id) => id,
            Self::Unnamed => "unresolved",
        }
    }
}

/// Whether this caller may make this request.
///
/// **The one place the confinement flip changes.** The rule below rests on a
/// premise with an expiry date: a same-uid process can `exec` whatever it likes
/// *today*, so a launch request grants it nothing and gating `Open` would be
/// ceremony. After the flip a confined application cannot `exec` at all, and
/// this socket becomes its only route to starting anything - at which point the
/// request really is authority and this function is where that is said.
///
/// Named rather than inlined for exactly that reason: the change should be a
/// different rule in a function that already exists, not a rewrite of the serve
/// path to acquire one.
fn admits(request: &LaunchRequest, caller: &Caller) -> bool {
    match request {
        // Naming a specific application is a claim about the user's setup, and
        // "app X caused app Y to start" is only a sentence if X has a name.
        LaunchRequest::App { .. } => matches!(caller, Caller::Named(_)),
        // Opening a document grants nothing the caller lacks, and refusing an
        // unrecognised binary would only cost someone the ability to open a file.
        LaunchRequest::Open { .. } => true,
    }
}

/// What the ledger records about one request.
///
/// **No document appears here.** A launch audit that carried the file name would
/// put the user's documents in a log that exists to answer a different question,
/// and "who caused what to start" is answerable without them. The MIME type of
/// an unhandled request is kept, because it names a gap in the configuration
/// rather than a thing the user was reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLine {
    /// The attested caller.
    pub caller: String,
    /// The application that was started, when one was.
    pub started: Option<String>,
    /// A short, fixed outcome word, so the ledger is filterable.
    pub outcome: &'static str,
}

/// The decision for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// What to tell the caller.
    pub outcome: LaunchOutcome,
    /// What to spawn, when the answer is "start it". The host runs this; it is
    /// absent for every refusal, so a refusal cannot be spawned by mistake.
    pub launch: Option<Launch>,
    /// What to write to the ledger, before spawning.
    pub audit: AuditLine,
}

/// Decide one request.
///
/// The lookups are injected for the same reason they are in [`resolve`]: the
/// decision is testable without a filesystem, and the host keeps the I/O.
pub fn serve(
    request: &LaunchRequest,
    caller: &Caller,
    mimeapps: &[MimeApps],
    entry: impl Fn(&str) -> Option<Entry>,
    mime_of: impl Fn(&arlen_launch_contract::Target) -> Option<String>,
    confined: bool,
    has_profile: impl Fn(&str) -> bool,
) -> Served {
    let refuse = |outcome: LaunchOutcome, word: &'static str| Served {
        outcome,
        launch: None,
        audit: AuditLine {
            caller: caller.as_str().to_string(),
            started: None,
            outcome: word,
        },
    };

    if !admits(request, caller) {
        return refuse(LaunchOutcome::Refused, "refused:unresolved-caller");
    }

    match resolve(request, mimeapps, entry, mime_of, confined, has_profile) {
        Ok(launch) => {
            // The resolved application, which for an `Open` is not what the
            // caller named - it named a document.
            let started = started_app_id(&launch);
            Served {
                outcome: LaunchOutcome::Started {
                    app_id: started.clone(),
                },
                launch: Some(launch),
                audit: AuditLine {
                    caller: caller.as_str().to_string(),
                    started: Some(started),
                    outcome: "started",
                },
            }
        }
        Err(LaunchError::NoHandler { mime }) => {
            refuse(LaunchOutcome::NoHandler { mime }, "no-handler")
        }
        Err(LaunchError::UnknownApplication { app_id }) => {
            refuse(LaunchOutcome::UnknownApplication { app_id }, "unknown-app")
        }
        Err(LaunchError::MalformedEntry { app_id, reason }) => refuse(
            LaunchOutcome::MalformedEntry {
                app_id,
                reason: reason.to_string(),
            },
            "malformed-entry",
        ),
        Err(LaunchError::NothingToRun { app_id }) => refuse(
            LaunchOutcome::MalformedEntry {
                app_id,
                // Distinct from a parse failure to a reader, and the caller
                // shows this sentence rather than acting on it.
                reason: "its launcher entry has nothing to run without a document".to_string(),
            },
            "nothing-to-run",
        ),
    }
}

/// The application id a planned launch will run under.
///
/// For a confined launch it is the argument after `--app-id`; the unconfined
/// shape does not carry one, and the program name is the closest true answer -
/// claiming an app id the launch is not using would make the ledger say
/// something that did not happen.
fn started_app_id(launch: &Launch) -> String {
    match launch {
        Launch::Confined(argv) => argv
            .iter()
            .position(|a| a == "--app-id")
            .and_then(|i| argv.get(i + 1))
            .cloned()
            .unwrap_or_default(),
        Launch::Direct(argv) => argv.first().cloned().unwrap_or_default(),
    }
}

/// The ledger record for one launch.
///
/// [`AuditKind::AppAction`] and the notification daemon's shape: a fixed
/// subject, the coarse app ids in `node_types`, the disposition in `outcome`.
/// A launch is observed, non-AI system activity, and the record carries who
/// asked and what started - never what was opened.
pub fn launch_event(line: &AuditLine) -> audit_proto::IngestRequest {
    let mut apps = vec![line.caller.clone()];
    apps.extend(line.started.clone());
    audit_proto::IngestRequest {
        kind: audit_proto::AuditKind::AppAction,
        structural: audit_proto::StructuralRecord {
            subject: "launch.request".to_string(),
            node_types: apps,
            outcome: line.outcome.to_string(),
            ..Default::default()
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// The marker that says the ledger is missing entries, and how many.
///
/// A warning in the journal is not surfacing. A missing search result is
/// visibly wrong; a missing audit line is invisible, and what it leaves behind
/// is a **complete-looking history that is not complete**. Everything the system
/// claims about being answerable rests on that ledger, so a hole in it has to be
/// stated inside the ledger rather than in a log nobody reads.
///
/// `result_count` is how many launches went unrecorded and `duration_ms` how
/// long the gap lasted, which with the entry's own append time gives both ends
/// of the window. A reader of the transparency surface can then say "there is a
/// gap here" instead of quietly showing less than happened.
pub fn unrecorded_gap_event(dropped: u64, span_ms: u64) -> audit_proto::IngestRequest {
    audit_proto::IngestRequest {
        kind: audit_proto::AuditKind::AppAction,
        structural: audit_proto::StructuralRecord {
            subject: "launch.unrecorded".to_string(),
            result_count: Some(dropped),
            duration_ms: Some(span_ms),
            outcome: "gap".to_string(),
            ..Default::default()
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_launch_contract::Target;

    /// These tests always supply a type, so a call to the sniffer would be a
    /// bug rather than a fallback.
    fn no_sniff(_: &Target) -> Option<String> {
        None
    }

    fn entry_of(app_id: &str, exec: &str) -> Entry {
        Entry {
            app_id: app_id.to_string(),
            exec: exec.to_string(),
            icon: None,
            name: None,
            desktop_file: None,
        }
    }

    fn catalog(id: &str) -> Option<Entry> {
        match id {
            "viewer.desktop" => Some(entry_of("org.arlen.Viewer", "viewer %f")),
            _ => None,
        }
    }

    fn handlers() -> Vec<MimeApps> {
        vec![super::super::mimeapps::parse(
            "[Default Applications]\nimage/png=viewer.desktop;\n",
        )]
    }

    fn open() -> LaunchRequest {
        LaunchRequest::Open {
            target: Target {
                uri: "file:///home/u/holiday.png".into(),
                path: Some("/home/u/holiday.png".into()),
            },
            mime: Some("image/png".into()),
        }
    }

    #[test]
    fn opening_a_document_works_for_a_caller_with_no_name() {
        let s = serve(
            &open(),
            &Caller::Unnamed,
            &handlers(),
            catalog,
            no_sniff,
            false,
            |_| true,
        );
        assert!(s.launch.is_some());
        assert_eq!(s.audit.caller, "unresolved");
        assert_eq!(s.audit.outcome, "started");
    }

    /// The claim-making shape needs a name to be a sentence in the ledger.
    #[test]
    fn naming_an_application_needs_a_named_caller() {
        let r = LaunchRequest::App {
            app_id: "viewer.desktop".into(),
            targets: vec![],
        };
        let s = serve(&r, &Caller::Unnamed, &[], catalog, no_sniff, false, |_| {
            true
        });
        assert_eq!(s.outcome, LaunchOutcome::Refused);
        assert!(s.launch.is_none());
        assert_eq!(s.audit.outcome, "refused:unresolved-caller");

        let ok = serve(
            &r,
            &Caller::Named("files".into()),
            &[],
            catalog,
            no_sniff,
            false,
            |_| true,
        );
        assert!(ok.launch.is_some());
        assert_eq!(ok.audit.caller, "files");
    }

    /// The field is optional so the callee can answer it, and this is that
    /// answer arriving: the caller said nothing, the service worked it out.
    #[test]
    fn a_request_without_a_type_gets_one_from_the_service() {
        let r = LaunchRequest::Open {
            target: Target {
                uri: "file:///home/u/holiday.png".into(),
                path: Some("/home/u/holiday.png".into()),
            },
            mime: None,
        };
        let sniff = |_: &Target| Some("image/png".to_string());
        let s = serve(
            &r,
            &Caller::Unnamed,
            &handlers(),
            catalog,
            sniff,
            false,
            |_| true,
        );
        assert!(s.launch.is_some());
        assert_eq!(s.audit.started.as_deref(), Some("viewer"));
    }

    /// And a target nothing can classify is "nothing opens this" rather than a
    /// failure with a cause the requester cannot act on.
    #[test]
    fn a_target_of_unknown_type_reads_as_no_handler() {
        let r = LaunchRequest::Open {
            target: Target {
                uri: "file:///home/u/mystery".into(),
                path: Some("/home/u/mystery".into()),
            },
            mime: None,
        };
        let s = serve(
            &r,
            &Caller::Unnamed,
            &handlers(),
            catalog,
            no_sniff,
            false,
            |_| true,
        );
        assert!(matches!(s.outcome, LaunchOutcome::NoHandler { .. }));
        assert!(s.launch.is_none());
    }

    /// A served request still causes a program to start, so an anonymous one
    /// must not read as though nothing did.
    #[test]
    fn an_unresolved_caller_is_written_out_rather_than_left_blank() {
        let s = serve(
            &open(),
            &Caller::Unnamed,
            &handlers(),
            catalog,
            no_sniff,
            false,
            |_| true,
        );
        assert_eq!(s.audit.caller, "unresolved");
        assert!(!s.audit.caller.is_empty());
        assert_eq!(s.audit.started.as_deref(), Some("viewer"));
    }

    /// The premise under `admits` expires at the confinement flip, when a
    /// confined app cannot `exec` and this socket becomes its only way to start
    /// anything. This pins today's rule so the change is visible as a change.
    #[test]
    fn todays_admission_rule_is_open_for_all_and_app_for_the_named() {
        let app = LaunchRequest::App {
            app_id: "viewer.desktop".into(),
            targets: vec![],
        };
        assert!(admits(&open(), &Caller::Unnamed));
        assert!(admits(&open(), &Caller::Named("x".into())));
        assert!(!admits(&app, &Caller::Unnamed));
        assert!(admits(&app, &Caller::Named("x".into())));
    }

    /// The point of point 3: the ledger says who caused what.
    #[test]
    fn the_ledger_names_both_ends() {
        let s = serve(
            &open(),
            &Caller::Named("org.arlen.Files".into()),
            &handlers(),
            catalog,
            no_sniff,
            true,
            |_| true,
        );
        assert_eq!(s.audit.caller, "org.arlen.Files");
        assert_eq!(s.audit.started.as_deref(), Some("org.arlen.Viewer"));
    }

    /// The ledger entry carries the two app ids and the disposition, and the
    /// document is not among them.
    #[test]
    fn the_ledger_entry_names_the_apps_and_nothing_else() {
        let s = serve(
            &open(),
            &Caller::Named("org.arlen.Files".into()),
            &handlers(),
            catalog,
            no_sniff,
            true,
            |_| true,
        );
        let event = launch_event(&s.audit);
        assert_eq!(event.kind, audit_proto::AuditKind::AppAction);
        assert_eq!(event.structural.subject, "launch.request");
        assert_eq!(
            event.structural.node_types,
            ["org.arlen.Files", "org.arlen.Viewer"]
        );
        assert_eq!(event.structural.outcome, "started");
        let rendered = format!("{:?}", event.structural);
        assert!(
            !rendered.contains("holiday"),
            "document in the record: {rendered}"
        );
    }

    /// A refusal still records who asked; only the started app is absent,
    /// because nothing started.
    #[test]
    fn a_refusal_records_the_caller_alone() {
        let r = LaunchRequest::App {
            app_id: "nope.desktop".into(),
            targets: vec![],
        };
        let s = serve(
            &r,
            &Caller::Unnamed,
            &handlers(),
            catalog,
            no_sniff,
            false,
            |_| true,
        );
        let event = launch_event(&s.audit);
        assert_eq!(event.structural.node_types, ["unresolved"]);
        assert_eq!(event.structural.outcome, "refused:unresolved-caller");
    }

    /// A gap has to be visible in the ledger, because a ledger that quietly
    /// shows less than happened looks exactly like one that shows everything.
    #[test]
    fn a_gap_marker_carries_how_many_and_how_long() {
        let e = unrecorded_gap_event(7, 4_200);
        assert_eq!(e.kind, audit_proto::AuditKind::AppAction);
        assert_eq!(e.structural.subject, "launch.unrecorded");
        assert_eq!(e.structural.result_count, Some(7));
        assert_eq!(e.structural.duration_ms, Some(4_200));
        assert_eq!(e.structural.outcome, "gap");
        // No app is named: the marker is about the ledger, not about a launch,
        // and listing the apps whose records were lost would be a record of
        // them by another route.
        assert!(e.structural.node_types.is_empty());
    }

    /// A launch audit is not a reading list.
    #[test]
    fn no_document_reaches_the_ledger() {
        let s = serve(
            &open(),
            &Caller::Named("x".into()),
            &handlers(),
            catalog,
            no_sniff,
            true,
            |_| true,
        );
        let line = format!("{:?}", s.audit);
        assert!(
            !line.contains("holiday"),
            "the document name is in the audit line: {line}"
        );
        assert!(!line.contains("/home/u"));
    }

    /// Every refusal is unspawnable by construction, not by the caller
    /// remembering to check the outcome first.
    #[test]
    fn a_refusal_carries_nothing_to_spawn() {
        for (r, word) in [
            (
                LaunchRequest::Open {
                    target: Target {
                        uri: "file:///x.zzz".into(),
                        path: Some("/x.zzz".into()),
                    },
                    mime: Some("application/x-nothing".into()),
                },
                "no-handler",
            ),
            (
                LaunchRequest::App {
                    app_id: "nope.desktop".into(),
                    targets: vec![],
                },
                "unknown-app",
            ),
        ] {
            let s = serve(
                &r,
                &Caller::Named("x".into()),
                &handlers(),
                catalog,
                no_sniff,
                false,
                |_| true,
            );
            assert!(s.launch.is_none());
            assert_eq!(s.audit.outcome, word);
            assert_eq!(s.audit.started, None);
        }
    }

    /// A missing default is a gap in the configuration, so the type stays in the
    /// answer where a document name would not.
    #[test]
    fn an_unhandled_type_is_named_in_the_outcome() {
        let r = LaunchRequest::Open {
            target: Target {
                uri: "file:///x.zzz".into(),
                path: Some("/x.zzz".into()),
            },
            mime: Some("application/x-nothing".into()),
        };
        let s = serve(
            &r,
            &Caller::Named("x".into()),
            &handlers(),
            catalog,
            no_sniff,
            false,
            |_| true,
        );
        assert_eq!(
            s.outcome,
            LaunchOutcome::NoHandler {
                mime: "application/x-nothing".into()
            }
        );
    }

    #[test]
    fn a_malformed_entry_reaches_the_caller_as_a_sentence() {
        let broken = |_: &str| Some(entry_of("org.x.Broken", "prog \"unclosed"));
        let r = LaunchRequest::App {
            app_id: "x.desktop".into(),
            targets: vec![],
        };
        let s = serve(
            &r,
            &Caller::Named("x".into()),
            &[],
            broken,
            no_sniff,
            false,
            |_| true,
        );
        match s.outcome {
            LaunchOutcome::MalformedEntry { app_id, reason } => {
                assert_eq!(app_id, "org.x.Broken");
                assert!(reason.contains("quote"), "unhelpful reason: {reason}");
            }
            other => panic!("expected a malformed entry, got {other:?}"),
        }
    }
}
