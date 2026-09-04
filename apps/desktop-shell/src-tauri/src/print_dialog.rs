//! The print dialog's three commands (`printing-plan.md` PRN-R3).
//!
//! An app prints through `org.freedesktop.portal.Print`. The portal holds the
//! document and asks this shell which printer and which options; the dialog puts
//! that question in front of a person; the answer goes back and the portal
//! prints. The app never touches a printer, only the result of a dialog somebody
//! drove - the same isolation the file picker has, and the reason the portal is
//! in the middle at all.
//!
//! THE DOCUMENT NEVER ARRIVES HERE. What crosses is a title, the app that asked
//! and a page count. Anything more would hand the shell a copy of a document an
//! app only asked to have printed.
//!
//! Blocking, one connection per call, on a blocking thread: the same shape the
//! Settings backend uses for the bottle daemon. A portal that is not running is a
//! failed connect, which the dialog reads as nothing pending - never as a print
//! that quietly went ahead.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use xdg_portal_arlen_protocol::print::{
    socket_path, DialogRequest, DialogResponse, PrintChoice, PrintRequest,
};

/// The largest answer this will read, matching the portal's own cap.
const MAX_FRAME: usize = 64 * 1024;

/// Ask the portal at `path` one question.
///
/// The path is a parameter rather than read in here so the tests can point at
/// one nothing is listening on without setting an environment variable the rest
/// of this crate's tests are reading at the same time.
fn ask(path: &std::path::Path, request: &DialogRequest) -> Result<DialogResponse, String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|e| format!("the print portal is not running ({}): {e}", path.display()))?;
    let body = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .and_then(|()| stream.flush())
        .map_err(|e| format!("the print portal stopped listening: {e}"))?;

    let mut len = [0u8; 4];
    stream
        .read_exact(&mut len)
        .map_err(|e| format!("the print portal did not answer: {e}"))?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err("the print portal answered with more than this reads".to_string());
    }
    let mut out = vec![0u8; len];
    stream
        .read_exact(&mut out)
        .map_err(|e| format!("the print portal stopped answering: {e}"))?;
    serde_json::from_slice(&out).map_err(|e| format!("the print portal answered oddly: {e}"))
}

/// Run one ask off the async runtime.
async fn round_trip(request: DialogRequest) -> Result<DialogResponse, String> {
    tokio::task::spawn_blocking(move || ask(&socket_path(), &request))
        .await
        .map_err(|e| format!("the print portal could not be asked: {e}"))?
}

/// What a poll's answer means. An unreachable portal is no pending print.
fn poll_answer(answer: Result<DialogResponse, String>) -> Option<PrintRequest> {
    match answer {
        Ok(DialogResponse::Pending { request }) => request,
        _ => None,
    }
}

/// What a submit's answer means.
fn submit_answer(answer: DialogResponse) -> Result<(), String> {
    match answer {
        DialogResponse::Taken => Ok(()),
        DialogResponse::Unknown => {
            Err("this print is no longer waiting, so nothing was printed".to_string())
        }
        DialogResponse::Pending { .. } => Err("the print portal answered something else".to_string()),
    }
}

/// What a cancel's answer means.
fn cancel_answer(answer: DialogResponse) -> Result<(), String> {
    match answer {
        DialogResponse::Taken | DialogResponse::Unknown => Ok(()),
        DialogResponse::Pending { .. } => Err("the print portal answered something else".to_string()),
    }
}

/// The next document waiting to be printed, or nothing.
///
/// An unreachable portal answers `None` rather than an error: there is no
/// pending print if nothing can be printing, and a dialog that popped an error
/// box every time the portal was absent would be noise on every boot without one.
#[tauri::command]
pub async fn poll_print_request() -> Result<Option<PrintRequest>, String> {
    Ok(poll_answer(round_trip(DialogRequest::Poll).await))
}

/// Print it, with the printer and options the person chose.
///
/// An error here means nothing was printed, and the dialog stays open saying so.
/// `Unknown` is an error for the same reason: the portal gave up waiting, so the
/// job the person just pressed Print on is not going to happen, and closing the
/// dialog would tell them it did.
#[tauri::command]
pub async fn submit_print(id: String, settings: PrintChoice) -> Result<(), String> {
    submit_answer(round_trip(DialogRequest::Submit { id, settings }).await?)
}

/// Decline it. Nothing is printed.
///
/// `Unknown` is fine here and not an error: the portal having already given up
/// and the person having declined reach the same end, which is that no document
/// was printed.
#[tauri::command]
pub async fn cancel_print(id: String) -> Result<(), String> {
    cancel_answer(round_trip(DialogRequest::Cancel { id }).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_portal_is_not_an_error_to_a_poll() {
        // A shell on a machine with no portal polls this on every boot; an error
        // box there would be noise on a machine that simply is not printing.
        let dir = tempfile::tempdir().unwrap();
        let answer = ask(&dir.path().join("absent.sock"), &DialogRequest::Poll);
        assert!(answer.is_err(), "the connect failed");
        assert!(poll_answer(answer).is_none());
    }

    #[test]
    fn a_missing_portal_says_it_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let e = ask(&dir.path().join("absent.sock"), &DialogRequest::Poll).unwrap_err();
        assert!(e.contains("not running"), "{e}");
    }

    #[test]
    fn a_print_the_portal_gave_up_on_is_not_reported_as_printed() {
        let e = submit_answer(DialogResponse::Unknown).unwrap_err();
        assert!(e.contains("nothing was printed"), "{e}");
        assert!(submit_answer(DialogResponse::Taken).is_ok());
    }

    #[test]
    fn a_cancel_reaches_the_same_end_either_way() {
        // Declined, or the portal already gave up: no document was printed, and
        // the dialog is closed because the person said no.
        assert!(cancel_answer(DialogResponse::Taken).is_ok());
        assert!(cancel_answer(DialogResponse::Unknown).is_ok());
    }
}
