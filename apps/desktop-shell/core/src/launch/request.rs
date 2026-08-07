//! What a caller may ask for, and turning that into a launch.
//!
//! **There is no variant carrying a command line.** That is the whole shape of
//! this type: a command line in a launch request is arbitrary code execution
//! wearing a request's clothes, and once one exists the confinement flag is
//! advisory rather than enforced. A caller names an application, or names a
//! document and lets the system decide what opens it. Making that structural
//! rather than a rule in a comment is why the request is an enum here instead of
//! a string somewhere.
//!
//! This is also where the pieces meet: [`super::mimeapps`] answers which
//! application, [`super::exec`] turns its entry into argv, [`super::plan`]
//! decides whether that argv goes through `arlen-run`. Resolution and launch in
//! one component was the point - the gap they close is that the portal knew the
//! URI, `xdg-open` knew the handler and `arlen-run` needed the app id, and
//! nobody held all three.
//!
//! Pure. The lookups that touch the disk - what MIME type a URI has, what a
//! desktop entry says - are injected, so the composition is testable without a
//! filesystem and the host keeps the I/O.

use super::exec::{expand_exec, ExecContext, ExecError, Target};
use super::mimeapps::{default_handler, MimeApps};
use super::plan::{plan, Launch, NotLaunchable};

/// What a caller wants to happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchRequest {
    /// Start this application, optionally with documents. The rare case: an app
    /// that wants a *specific* application rather than whatever opens a document
    /// is worth a reason, and the honest default is that it does not need one.
    App {
        /// The application to start.
        app_id: String,
        /// Documents to hand it. Usually empty.
        targets: Vec<Target>,
    },
    /// Open this document with whatever the user's configuration says opens it.
    /// Nearly every real case.
    Open {
        /// The document.
        target: Target,
        /// Its MIME type, which the host determines - that is shared-mime-info's
        /// job, not this module's.
        mime: String,
    },
}

/// A desktop entry, reduced to what launching needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The application id `arlen-run` keys the permission profile on.
    pub app_id: String,
    /// The entry's `Exec`, verbatim, field codes intact.
    pub exec: String,
    /// `Icon`, for `%i`.
    pub icon: Option<String>,
    /// The translated `Name`, for `%c`.
    pub name: Option<String>,
    /// Where the entry file is, for `%k`.
    pub desktop_file: Option<String>,
}

/// Why a request could not become a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// Nothing is configured to open this type. Distinct from a broken handler:
    /// the honest answer is "you have not chosen one", which is a different thing
    /// to tell a user than "it failed".
    NoHandler {
        /// The type nothing claimed.
        mime: String,
    },
    /// The handler names an application whose entry could not be read.
    UnknownApplication {
        /// The application that was named.
        app_id: String,
    },
    /// The entry's `Exec` is not a valid command line per the desktop-entry
    /// spec, so the application's own packaging is at fault rather than the
    /// request.
    MalformedEntry {
        /// Which application.
        app_id: String,
        /// What is wrong with it.
        reason: ExecError,
    },
    /// The entry parses but leaves nothing to run.
    NothingToRun {
        /// Which application.
        app_id: String,
    },
}

/// Turn a request into the argv to spawn.
///
/// `mimeapps` are the parsed handler files in precedence order; `entry` reads a
/// desktop entry by its id, and answers `None` for one that is not installed,
/// which is also what makes the handler lookup skip a stale default.
///
/// `confined` is the launcher flag, and it reaches [`plan`] unchanged - this
/// composes the pieces, it does not add a policy of its own.
pub fn resolve(
    request: &LaunchRequest,
    mimeapps: &[MimeApps],
    entry: impl Fn(&str) -> Option<Entry>,
    confined: bool,
) -> Result<Launch, LaunchError> {
    let (app, targets) = match request {
        LaunchRequest::App { app_id, targets } => {
            let e = entry(app_id).ok_or_else(|| LaunchError::UnknownApplication {
                app_id: app_id.clone(),
            })?;
            (e, targets.clone())
        }
        LaunchRequest::Open { target, mime } => {
            let id = default_handler(mimeapps, mime, |id| entry(id).is_some())
                .ok_or_else(|| LaunchError::NoHandler { mime: mime.clone() })?;
            // The handler lookup already required the entry to exist, so an
            // absent one here is a race rather than a miss - reported as
            // unknown either way, which is true and does not invent a cause.
            let e = entry(&id).ok_or(LaunchError::UnknownApplication { app_id: id })?;
            (e, vec![target.clone()])
        }
    };

    let argv = expand_exec(
        &app.exec,
        &ExecContext {
            targets: &targets,
            icon: app.icon.as_deref(),
            name: app.name.as_deref(),
            desktop_file: app.desktop_file.as_deref(),
        },
    )
    // `Empty` is not a malformed entry: the syntax was fine, there was just
    // nothing left after the codes that had nothing to fill them. An entry whose
    // whole command line is `%f`, launched without a document, is that case, and
    // calling it malformed would point a packaging complaint at a working entry.
    .map_err(|reason| match reason {
        ExecError::Empty => LaunchError::NothingToRun {
            app_id: app.app_id.clone(),
        },
        reason => LaunchError::MalformedEntry {
            app_id: app.app_id.clone(),
            reason,
        },
    })?;

    // `expand_exec` has already refused an empty result, so this arm is not
    // reachable from here today. Mapped rather than unwrapped because that is a
    // fact about the current expander, not a guarantee of the signature.
    plan(confined, Some(&app.app_id), &argv).map_err(|e| match e {
        NotLaunchable::EmptyArgv => LaunchError::NothingToRun {
            app_id: app.app_id.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> Target {
        Target {
            uri: format!("file://{path}"),
            path: Some(path.to_string()),
        }
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

    /// One installed application, `viewer.desktop`, that takes a file.
    fn catalog(id: &str) -> Option<Entry> {
        match id {
            "viewer.desktop" => Some(entry_of("org.arlen.Viewer", "viewer %f")),
            "editor.desktop" => Some(entry_of("org.arlen.Editor", "editor")),
            _ => None,
        }
    }

    fn handlers(text: &str) -> Vec<MimeApps> {
        vec![super::super::mimeapps::parse(text)]
    }

    #[test]
    fn opening_a_document_finds_its_handler_and_passes_it_the_file() {
        let m = handlers("[Default Applications]\nimage/png=viewer.desktop;\n");
        let r = LaunchRequest::Open {
            target: file("/tmp/a.png"),
            mime: "image/png".into(),
        };
        assert_eq!(
            resolve(&r, &m, catalog, false),
            Ok(Launch::Direct(vec!["viewer".into(), "/tmp/a.png".into()]))
        );
    }

    #[test]
    fn naming_an_application_starts_it_without_a_document() {
        let r = LaunchRequest::App {
            app_id: "editor.desktop".into(),
            targets: vec![],
        };
        assert_eq!(
            resolve(&r, &[], catalog, false),
            Ok(Launch::Direct(vec!["editor".into()]))
        );
    }

    /// The flag reaches `plan` unchanged, and the app id is the entry's own -
    /// not the desktop id the caller happened to name.
    #[test]
    fn a_confined_launch_carries_the_entrys_app_id_not_the_desktop_id() {
        let m = handlers("[Default Applications]\nimage/png=viewer.desktop;\n");
        let r = LaunchRequest::Open {
            target: file("/tmp/a.png"),
            mime: "image/png".into(),
        };
        assert_eq!(
            resolve(&r, &m, catalog, true),
            Ok(Launch::Confined(vec![
                "arlen-run".into(),
                "--app-id".into(),
                "org.arlen.Viewer".into(),
                "--".into(),
                "viewer".into(),
                "/tmp/a.png".into(),
            ]))
        );
    }

    /// "You have not chosen one" is a different thing to tell a user than
    /// "it failed", so it is a different error.
    #[test]
    fn a_type_with_no_handler_says_so_rather_than_failing_vaguely() {
        assert_eq!(
            resolve(
                &LaunchRequest::Open {
                    target: file("/tmp/a.xyz"),
                    mime: "application/x-nothing".into(),
                },
                &[],
                catalog,
                false
            ),
            Err(LaunchError::NoHandler {
                mime: "application/x-nothing".into()
            })
        );
    }

    /// A default naming something uninstalled falls through to the next
    /// candidate rather than becoming a launch failure.
    #[test]
    fn an_uninstalled_default_does_not_shadow_a_working_one() {
        let m = handlers("[Default Applications]\nimage/png=gone.desktop;viewer.desktop;\n");
        let r = LaunchRequest::Open {
            target: file("/tmp/a.png"),
            mime: "image/png".into(),
        };
        assert_eq!(
            resolve(&r, &m, catalog, false),
            Ok(Launch::Direct(vec!["viewer".into(), "/tmp/a.png".into()]))
        );
    }

    #[test]
    fn naming_an_application_nobody_installed_is_its_own_error() {
        assert_eq!(
            resolve(
                &LaunchRequest::App {
                    app_id: "nope.desktop".into(),
                    targets: vec![]
                },
                &[],
                catalog,
                false
            ),
            Err(LaunchError::UnknownApplication {
                app_id: "nope.desktop".into()
            })
        );
    }

    /// A packaging fault reads as a packaging fault, with the application named.
    #[test]
    fn a_malformed_exec_names_the_application_and_the_reason() {
        let broken = |_: &str| Some(entry_of("org.x.Broken", "prog \"unclosed"));
        assert_eq!(
            resolve(
                &LaunchRequest::App {
                    app_id: "x.desktop".into(),
                    targets: vec![]
                },
                &[],
                broken,
                false
            ),
            Err(LaunchError::MalformedEntry {
                app_id: "org.x.Broken".into(),
                reason: ExecError::UnterminatedQuote,
            })
        );
    }

    #[test]
    fn an_exec_that_expands_to_nothing_is_not_a_launch() {
        let empty = |_: &str| Some(entry_of("org.x.Empty", "%f"));
        assert_eq!(
            resolve(
                &LaunchRequest::App {
                    app_id: "x.desktop".into(),
                    targets: vec![]
                },
                &[],
                empty,
                false
            ),
            Err(LaunchError::NothingToRun {
                app_id: "org.x.Empty".into()
            })
        );
    }

    /// The property the enum exists for: a document is data on the way through,
    /// whatever its name looks like.
    #[test]
    fn a_document_named_like_a_command_is_one_argument() {
        let m = handlers("[Default Applications]\ntext/plain=viewer.desktop;\n");
        let r = LaunchRequest::Open {
            target: file("/tmp/; rm -rf ~"),
            mime: "text/plain".into(),
        };
        assert_eq!(
            resolve(&r, &m, catalog, false),
            Ok(Launch::Direct(vec![
                "viewer".into(),
                "/tmp/; rm -rf ~".into()
            ]))
        );
    }
}
