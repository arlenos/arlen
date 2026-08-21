//! Printing a file, through the portal rather than around it.
//!
//! `daemons/xdg-portal/daemon/src/interfaces/print.rs` has been able to hand a
//! document to CUPS since it was written, and until 19 August nothing in the
//! system ever called it: Settings could list printers and set their options,
//! but no app could print anything. The five operations were reachable and the
//! one that matters was not.
//!
//! IN THE PLUGIN, not in an app, because every app that opens a file has the
//! same claim on it and the code is fd handling and bus etiquette rather than
//! anything about pictures or text. The viewer had it first and for one day; a
//! second copy in the editor would have been the moment to notice that.
//!
//! An app does not talk to our backend directly - it talks to the standard
//! frontend (`org.freedesktop.portal.Desktop`), which authenticates the caller
//! and dispatches to whichever backend is installed. Our impl refuses any
//! sender that is not the frontend, so this route is the only one that works,
//! which is the intended shape and not an obstacle.
//!
//! What crosses the bus is a FILE DESCRIPTOR, not a path. The portal reads the
//! document through the fd we hand it, so a print of a file the app can read
//! cannot be redirected by a path race, and the print service never needs read
//! access to the user's home.

use std::collections::HashMap;

use serde::Serialize;
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

/// How a print attempt ended.
///
/// Distinct variants because the three endings are three different things to a
/// person: the document went to the printer, they changed their mind, or the
/// dialog is still sitting there. An app that collapses these into "printing
/// failed" is lying about two of them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum PrintOutcome {
    /// The portal accepted the document.
    Sent,
    /// The person closed the dialog without printing.
    Cancelled,
    /// The portal ended the request some other way, without saying more.
    Refused,
    /// No answer within the wait. The dialog is presumably still open; nothing
    /// has been printed and nothing has been cancelled.
    NoAnswerYet,
}

/// How long to wait for the person to answer the print dialog.
///
/// Long, because a print dialog waits for a human who may be choosing a tray or
/// walking to the printer, and reporting "no answer" after ten seconds would be
/// wrong far more often than right. Bounded all the same, so a dialog that
/// never appears cannot leave the call pending forever.
const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(180);

/// The path the portal will use for this request, worked out in advance.
///
/// The response arrives as a signal on a request object, and a client that
/// subscribes only after the call can miss an answer that comes back
/// immediately - the portal is entitled to answer before the method returns.
/// The specification exists for this: hand it a `handle_token`, and the path is
/// then derivable from the token and our own unique name, so the subscription
/// can be in place before the request is made.
fn request_path(unique_name: &str, token: &str) -> String {
    // ":1.42" -> "1_42": the leading colon goes, dots become underscores.
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Print",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait PortalPrint {
    #[zbus(name = "Print")]
    fn print(
        &self,
        parent_window: &str,
        title: &str,
        fd: Fd<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop"
)]
trait PortalRequest {
    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}

/// Why a print did not start, as a word rather than as a sentence.
///
/// `PrintOutcome` already speaks in tokens for the ways a print ENDS, and the
/// windows translate those. The ways it fails to BEGIN were English strings -
/// "no print portal: ...", "no session bus: ..." - and they reach the same
/// windows, which then show a German frame around an English cause. On an image
/// with no printing stack at all that is not an edge case: it is what every
/// print does.
///
/// The detail is kept on each variant. Whoever reads it is usually the one
/// person who can act on it, and dropping what the layer below said would leave
/// them with less than they have now.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
pub enum PrintProblem {
    /// The file itself could not be opened to send.
    FileUnreadable { message: String },
    /// There is no session bus to ask.
    NoBus { message: String },
    /// The bus is there and the print portal is not - the shape of a machine
    /// with no printing set up.
    NoPortal { message: String },
    /// The portal was reached and would not take the document.
    PortalRefused { message: String },
    /// Anything else on the way: the request path, the answer watch, an answer
    /// that would not parse.
    Other { message: String },
}

impl PrintProblem {
    /// The wire form the windows read. A serializer failure is not worth a
    /// second vocabulary; the windows show an unrecognised answer verbatim.
    fn wire(self) -> String {
        serde_json::to_string(&self).unwrap_or_else(|_| "unserialisable".to_string())
    }
}

/// Print `path`, and report how it ended.
///
/// The title is what the print queue shows for the job, so it is the file's own
/// name rather than anything of ours.
#[tauri::command]
pub async fn print_file(path: String) -> Result<PrintOutcome, String> {
    let file = std::fs::File::open(&path).map_err(|e| PrintProblem::FileUnreadable { message: e.to_string() }.wire())?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());

    let connection = zbus::Connection::session()
        .await
        .map_err(|e| PrintProblem::NoBus { message: e.to_string() }.wire())?;
    let unique = connection
        .unique_name()
        .map(|n| n.to_string())
        .ok_or_else(|| PrintProblem::Other { message: "the session bus gave us no name".to_string() }.wire())?;

    // A token unique to this attempt, so two prints in a row cannot land on one
    // another's request object.
    let token = format!("arlen_viewers_{}", std::process::id());
    let token = format!("{token}_{}", now_millis());
    let path_for_reply = request_path(&unique, &token);

    let reply = PortalRequestProxy::builder(&connection)
        .path(path_for_reply.as_str())
        .map_err(|e| PrintProblem::Other { message: format!("bad request path: {e}") }.wire())?
        .build()
        .await
        .map_err(|e| PrintProblem::Other { message: format!("could not watch for the answer: {e}") }.wire())?;
    // Subscribed BEFORE the call, deliberately: see `request_path`.
    let mut answers = reply
        .receive_response()
        .await
        .map_err(|e| PrintProblem::Other { message: format!("could not watch for the answer: {e}") }.wire())?;

    let printer = PortalPrintProxy::new(&connection)
        .await
        .map_err(|e| PrintProblem::NoPortal { message: e.to_string() }.wire())?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    printer
        .print("", &name, Fd::from(&file), options)
        .await
        .map_err(|e| PrintProblem::PortalRefused { message: e.to_string() }.wire())?;

    match tokio::time::timeout(ANSWER_WAIT, futures_util::StreamExt::next(&mut answers)).await {
        Ok(Some(signal)) => {
            let args = signal
                .args()
                .map_err(|e| PrintProblem::Other { message: format!("the portal's answer made no sense: {e}") }.wire())?;
            Ok(match args.response {
                0 => PrintOutcome::Sent,
                1 => PrintOutcome::Cancelled,
                _ => PrintOutcome::Refused,
            })
        }
        // The stream ended without an answer: the portal went away mid-request.
        Ok(None) => Err(PrintProblem::Other {
            message: "the print portal stopped answering".to_string(),
        }
        .wire()),
        Err(_) => Ok(PrintOutcome::NoAnswerYet),
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The derivation is the whole reason the answer cannot be missed, and it is
    /// a string transformation with two easy mistakes in it: keeping the colon,
    /// or leaving the dots. Either produces a path the portal never signals on,
    /// and the symptom is every print reporting "no answer yet" while printing
    /// perfectly well.
    #[test]
    fn the_request_path_is_derived_the_way_the_portal_derives_it() {
        assert_eq!(
            request_path(":1.42", "tok"),
            "/org/freedesktop/portal/desktop/request/1_42/tok"
        );
        assert_eq!(
            request_path(":1.1024", "arlen_viewers_9_1"),
            "/org/freedesktop/portal/desktop/request/1_1024/arlen_viewers_9_1"
        );
    }

    /// Two prints from one process must not share a request object; if they did,
    /// the first answer would be read as the second's.
    #[test]
    fn two_attempts_do_not_share_a_request_object() {
        let a = format!("arlen_viewers_{}_{}", std::process::id(), 1u128);
        let b = format!("arlen_viewers_{}_{}", std::process::id(), 2u128);
        assert_ne!(request_path(":1.5", &a), request_path(":1.5", &b));
    }

    /// The frontend renders on the tag, so a rename here silently turns a
    /// cancelled print into an unhandled case.
    #[test]
    fn the_outcomes_serialise_as_the_frontend_reads_them() {
        let json = |o: PrintOutcome| serde_json::to_value(o).unwrap()["outcome"].clone();
        assert_eq!(json(PrintOutcome::Sent), "sent");
        assert_eq!(json(PrintOutcome::Cancelled), "cancelled");
        assert_eq!(json(PrintOutcome::Refused), "refused");
        assert_eq!(json(PrintOutcome::NoAnswerYet), "no-answer-yet");
    }

    /// The words the windows switch on. Pinned, because a window that stops
    /// recognising one of these does not fail: it falls back to showing the raw
    /// answer, which is the state this vocabulary exists to end.
    #[test]
    fn a_problem_travels_as_a_word_and_keeps_its_detail() {
        let wire = PrintProblem::NoPortal { message: "no such name".into() }.wire();
        assert!(wire.contains("\"problem\":\"no-portal\""), "{wire}");
        assert!(wire.contains("no such name"), "{wire}");

        for (p, word) in [
            (PrintProblem::FileUnreadable { message: String::new() }, "file-unreadable"),
            (PrintProblem::NoBus { message: String::new() }, "no-bus"),
            (PrintProblem::PortalRefused { message: String::new() }, "portal-refused"),
            (PrintProblem::Other { message: String::new() }, "other"),
        ] {
            assert!(p.wire().contains(word), "{word}");
        }
    }
}
