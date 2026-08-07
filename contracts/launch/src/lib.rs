//! The launch request contract: what one component may ask another to start.
//!
//! Three components need this vocabulary - the shell, which serves the launch
//! socket; the portal, which stops spawning `xdg-open` and asks instead; and the
//! apps, whose Open-With and per-app-settings handoffs are launch requests too.
//! Shared here rather than in any one of them, because a wire type that lives in
//! one participant's crate is a dependency the other participants should not
//! have.
//!
//! **[`LaunchRequest`] cannot express a command line.** That is the point of it
//! being a type at all. A command line in a launch request is arbitrary code
//! execution wearing a request's clothes, and the moment one exists the
//! confinement flag is advisory: whoever can name a program can name
//! `sh -c ...` and confine nothing. A caller names an application, or names a
//! document and lets the system decide what opens it. Three callers remembering
//! a rule is a convention; a variant that does not exist is a guarantee.
//!
//! The resolution and the launch itself live in the shell, together, because the
//! gap this closes is that the portal knew the URI, `xdg-open` knew the handler
//! and `arlen-run` needed the app id, and nobody held all three.

use serde::{Deserialize, Serialize};

/// A document, in the two forms a desktop entry's field codes want.
///
/// Both are carried because an application declares which it takes: `%u` gets
/// the URI, `%f` the local path, and an application that only handles local
/// files cannot open a remote document at all. Deciding that at the callee, from
/// the entry, is the only place that knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The URI, for `%u` / `%U`.
    pub uri: String,
    /// The local path, for `%f` / `%F`. Absent for a document that is not a
    /// local file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What a caller is asking for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LaunchRequest {
    /// Start a named application, optionally handing it documents.
    ///
    /// The rare case, deliberately. An application that wants a *specific* other
    /// application rather than whatever opens a document is making a claim about
    /// the user's setup, and the honest default is that it does not need to.
    App {
        /// The desktop id of the application to start.
        app_id: String,
        /// Documents to hand it. Usually empty.
        #[serde(default)]
        targets: Vec<Target>,
    },
    /// Open a document with whatever the user's configuration says opens it.
    /// Nearly every real case, and the one that needs no claim about the setup.
    Open {
        /// The document.
        target: Target,
        /// Its MIME type. The caller supplies it because MIME detection is
        /// shared-mime-info's job and the caller usually has the answer already;
        /// a caller that does not can ask the type separately rather than have
        /// this interface grow a sniffing mode.
        mime: String,
    },
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum LaunchOutcome {
    /// The application was started.
    Started {
        /// Which application, resolved - not necessarily the one the caller
        /// named, since an `Open` request names a document.
        app_id: String,
    },
    /// Nothing is configured to open this type.
    ///
    /// Distinct from a failure on purpose: "you have not chosen a handler" is a
    /// different thing to tell someone than "it did not work", and collapsing
    /// them is how a missing default reads as a broken application.
    NoHandler {
        /// The type nothing claimed.
        mime: String,
    },
    /// The named application is not installed, or its entry could not be read.
    UnknownApplication {
        /// What was named.
        app_id: String,
    },
    /// The application's own launcher entry is not a valid command line, so its
    /// packaging is at fault rather than the request. Carried as a sentence
    /// because the caller shows it and cannot act on a code.
    MalformedEntry {
        /// Which application.
        app_id: String,
        /// What is wrong with the entry.
        reason: String,
    },
    /// The request was refused. The reason is deliberately coarse: a caller
    /// learning exactly which check it failed learns how to pass it.
    Refused,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(r: &LaunchRequest) -> LaunchRequest {
        serde_json::from_str(&serde_json::to_string(r).unwrap()).unwrap()
    }

    #[test]
    fn a_request_survives_the_wire() {
        let r = LaunchRequest::Open {
            target: Target {
                uri: "file:///tmp/a.png".into(),
                path: Some("/tmp/a.png".into()),
            },
            mime: "image/png".into(),
        };
        assert_eq!(round_trip(&r), r);
    }

    #[test]
    fn an_app_request_defaults_to_no_documents() {
        let parsed: LaunchRequest =
            serde_json::from_str(r#"{"kind":"app","app_id":"org.x.App"}"#).unwrap();
        assert_eq!(
            parsed,
            LaunchRequest::App {
                app_id: "org.x.App".into(),
                targets: vec![]
            }
        );
    }

    /// A remote document has no local path, and inventing one would let an
    /// application that only takes `%f` be handed something that is not a file.
    #[test]
    fn a_target_without_a_path_stays_without_one() {
        let t = Target {
            uri: "https://example.org/x".into(),
            path: None,
        };
        let back: Target = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back.path, None);
        assert!(!serde_json::to_string(&t).unwrap().contains("path"));
    }

    /// The guarantee this type exists for: there is no way to ask for a command
    /// line, so a request that tries is not a request.
    #[test]
    fn a_command_line_is_not_a_representable_request() {
        for body in [
            r#"{"kind":"exec","command":"sh -c 'rm -rf ~'"}"#,
            r#"{"kind":"app","app_id":"x","command":"sh -c x"}"#,
            r#"{"command":"sh -c x"}"#,
        ] {
            assert!(
                serde_json::from_str::<LaunchRequest>(body).is_err()
                    || !serde_json::to_string(&serde_json::from_str::<LaunchRequest>(body).unwrap())
                        .unwrap()
                        .contains("command"),
                "a command line survived deserialisation: {body}"
            );
        }
    }

    #[test]
    fn an_outcome_survives_the_wire() {
        let o = LaunchOutcome::NoHandler {
            mime: "application/x-nothing".into(),
        };
        let back: LaunchOutcome = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert_eq!(back, o);
    }
}
