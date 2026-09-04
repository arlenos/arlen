//! The print dialog handback (`printing-plan.md` PRN-R2/R3).
//!
//! WHY THIS EXISTS AT ALL. The Print backend prints; until now it never asked
//! anybody. `PreparePrint` staged whatever settings the calling app sent and
//! `Print` sent the document to the default printer, which means an app chose the
//! printer and the person never saw a dialog. §2 of the plan is explicit that the
//! ARLEN dialog chooses, not the app - that is the whole isolation property, the
//! same one the file picker has - and Tim settled the dialog's form on 14 June:
//! first-party, portal-mediated, on the kit's canon.
//!
//! THE TRAFFIC RUNS THE OTHER WAY FROM THE PICKER, which is why this is not the
//! picker's IPC with different messages. The picker-ui is a subprocess the daemon
//! spawns per request and pushes to. The print dialog lives in the shell, which is
//! already running and cannot be spawned; so the shell polls, the portal answers
//! with whatever is waiting, and the portal is the one that blocks.
//!
//! THE DOCUMENT NEVER CROSSES. The portal holds the bytes and prints them; the
//! shell is told a title, an app and a page count so it can ask its question.
//! Handing the dialog the document would give the shell a copy of something an
//! app only ever asked to have printed.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, Mutex, Notify};
use xdg_portal_arlen_protocol::print::{
    DialogRequest, DialogResponse, PrintChoice, PrintRequest,
};

/// The largest accepted frame. A poll is a word and an answer is a handful of
/// short strings; a larger declared length is refused before it is allocated.
const MAX_FRAME: usize = 64 * 1024;

/// What the dialog decided.
#[derive(Debug)]
pub enum Answer {
    /// Print it with these choices.
    Print(Box<PrintChoice>),
    /// The person declined, or the dialog went away without answering. Both mean
    /// nothing is printed, and the portal cannot tell them apart from here.
    Cancelled,
}

/// Documents waiting for somebody to choose.
#[derive(Default)]
pub struct PendingPrints {
    /// Woken when a document joins the queue, for the connections parked on
    /// `Await`. A notify rather than a channel per waiter: every waiter wants
    /// the same news and none of them wants a copy of it.
    arrived: std::sync::Arc<Notify>,
    /// Waiting to be shown, oldest first. A request leaves the queue when the
    /// shell takes it, so two dialogs never open on one document.
    queue: VecDeque<PrintRequest>,
    /// Where to send the answer, keyed by request id. Outlives the queue entry:
    /// the request is gone from the queue while the dialog is open, and the
    /// answer still has to reach the waiting print.
    answers: HashMap<String, oneshot::Sender<Answer>>,
}

impl PendingPrints {
    /// Queue a document and hand back the channel its answer will arrive on.
    pub fn register(&mut self, request: PrintRequest) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        self.answers.insert(request.id.clone(), tx);
        self.queue.push_back(request);
        // Every parked connection, not one: a shell that reconnected while
        // another was still winding down would otherwise be the one told.
        self.arrived.notify_waiters();
        rx
    }

    /// A handle to wait on for the next arrival.
    ///
    /// Cloned out rather than awaited under the lock, which is the whole point:
    /// a waiter holding the registry's mutex would stop the very registration it
    /// is waiting for.
    pub fn arrivals(&self) -> std::sync::Arc<Notify> {
        std::sync::Arc::clone(&self.arrived)
    }

    /// The next document to show, if any.
    pub fn take_next(&mut self) -> Option<PrintRequest> {
        self.queue.pop_front()
    }

    /// Deliver an answer. False when nothing is waiting under that id.
    pub fn answer(&mut self, id: &str, answer: Answer) -> bool {
        match self.answers.remove(id) {
            // A closed receiver means the print gave up waiting. Reporting that
            // as delivered would be a lie, and the shell shows the same "nothing
            // is waiting under that id" either way.
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }

    /// Forget a request whose print stopped waiting, so a dialog that answers
    /// late is told there is nothing there rather than being silently accepted.
    pub fn forget(&mut self, id: &str) {
        self.answers.remove(id);
        self.queue.retain(|r| r.id != id);
    }

    /// How many documents are waiting to be shown.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.queue.len()
    }
}

/// The shared registry the Print backend and the socket both hold.
pub type Shared = Arc<Mutex<PendingPrints>>;

/// Answer one dialog request.
pub async fn handle(shared: &Shared, request: DialogRequest) -> DialogResponse {
    let mut pending = shared.lock().await;
    match request {
        DialogRequest::Poll => DialogResponse::Pending {
            request: pending.take_next(),
        },
        // Handled by the caller, which has to release the lock before waiting.
        DialogRequest::Await => DialogResponse::Pending {
            request: pending.take_next(),
        },
        DialogRequest::Submit { id, settings } => {
            if pending.answer(&id, Answer::Print(Box::new(settings))) {
                DialogResponse::Taken
            } else {
                DialogResponse::Unknown
            }
        }
        DialogRequest::Cancel { id } => {
            if pending.answer(&id, Answer::Cancelled) {
                DialogResponse::Taken
            } else {
                DialogResponse::Unknown
            }
        }
    }
}

/// Serve one connection: one request, one answer, close.
pub async fn serve_connection(shared: &Shared, mut stream: UnixStream) {
    let mut len = [0u8; 4];
    if stream.read_exact(&mut len).await.is_err() {
        return;
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        tracing::warn!("print dialog: refusing a frame of {len} bytes");
        return;
    }
    let mut body = vec![0u8; len];
    if stream.read_exact(&mut body).await.is_err() {
        return;
    }
    let response = match serde_json::from_slice::<DialogRequest>(&body) {
        // The parked ask: answer the moment something arrives, or when this
        // connection goes away. Waiting happens with the registry's lock RELEASED
        // - holding it would block the registration being waited for - so the
        // queue is re-checked after each wake rather than trusted from before it.
        Ok(DialogRequest::Await) => loop {
            let arrivals = {
                let mut pending = shared.lock().await;
                if let Some(request) = pending.take_next() {
                    break DialogResponse::Pending {
                        request: Some(request),
                    };
                }
                pending.arrivals()
            };
            arrivals.notified().await;
        },
        Ok(request) => handle(shared, request).await,
        Err(e) => {
            tracing::warn!("print dialog: unreadable request: {e}");
            return;
        }
    };
    let Ok(out) = serde_json::to_vec(&response) else {
        return;
    };
    let _ = stream.write_all(&(out.len() as u32).to_be_bytes()).await;
    let _ = stream.write_all(&out).await;
    let _ = stream.flush().await;
}

/// Bind the dialog socket, replacing one a previous run left behind.
///
/// Only an existing SOCKET is removed: a regular file or symlink at that path
/// belongs to something else, and unlinking it would make this daemon a way to
/// delete a file it was pointed at.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if meta.file_type().is_socket() {
            let _ = std::fs::remove_file(path);
        }
    }
    let listener = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}

/// Accept dialog connections for the life of the daemon.
pub async fn run(listener: UnixListener, shared: Shared) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let shared = Arc::clone(&shared);
                tokio::spawn(async move { serve_connection(&shared, stream).await });
            }
            Err(e) => {
                tracing::warn!("print dialog: accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

/// How many pages a document has, as far as this can honestly tell.
///
/// ZERO MEANS NOT KNOWN, and it is a real answer rather than a failure. A PDF
/// that keeps its page tree in an object stream is compressed, and counting page
/// objects in the raw bytes finds nothing there; a proper count needs a PDF
/// parser, which is a dependency this daemon does not have and should not grow
/// for a number the dialog uses to size a pager. So an uncompressed document
/// counts and a compressed one says it does not know, which the dialog renders as
/// no page range rather than as "1 page".
#[must_use]
pub fn page_count(document: &[u8]) -> u32 {
    let mut count = 0u32;
    let needle = b"/Type";
    let mut i = 0;
    while let Some(at) = find(&document[i..], needle) {
        let after = i + at + needle.len();
        // `/Type /Page` and `/Type/Page`, but never `/Type /Pages`, which is the
        // tree node rather than a leaf.
        let rest = &document[after..];
        let rest = strip_space(rest);
        if rest.starts_with(b"/Page") {
            let tail = &rest[b"/Page".len()..];
            if !tail.starts_with(b"s") {
                count = count.saturating_add(1);
            }
        }
        i = after;
    }
    count
}

fn strip_space(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\n' || b[i] == b'\r' || b[i] == b'\t') {
        i += 1;
    }
    &b[i..]
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdg_portal_arlen_protocol::print::{Color, Duplex, RangeMode};

    fn request(id: &str) -> PrintRequest {
        PrintRequest {
            id: id.to_string(),
            title: "report.pdf".into(),
            app_id: "org.arlen.files".into(),
            app_name: "Files".into(),
            page_count: 3,
        }
    }

    fn choice() -> PrintChoice {
        PrintChoice {
            printer: "Office HP".into(),
            copies: 1,
            range_mode: RangeMode::All,
            range_text: String::new(),
            duplex: Duplex::OneSided,
            color: Color::Color,
            paper: "a4".into(),
        }
    }

    #[tokio::test]
    async fn a_document_is_shown_once_and_its_answer_reaches_the_waiting_print() {
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let rx = shared.lock().await.register(request("p1"));

        let first = handle(&shared, DialogRequest::Poll).await;
        assert!(matches!(first, DialogResponse::Pending { request: Some(r) } if r.id == "p1"));
        // Taken off the queue, so a second dialog does not open on it.
        assert!(matches!(
            handle(&shared, DialogRequest::Poll).await,
            DialogResponse::Pending { request: None }
        ));

        let taken = handle(
            &shared,
            DialogRequest::Submit {
                id: "p1".into(),
                settings: choice(),
            },
        )
        .await;
        assert_eq!(taken, DialogResponse::Taken);
        assert!(matches!(rx.await.unwrap(), Answer::Print(_)));
    }

    #[tokio::test]
    async fn a_cancel_reaches_the_print_as_a_cancel() {
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let rx = shared.lock().await.register(request("p2"));
        assert_eq!(
            handle(&shared, DialogRequest::Cancel { id: "p2".into() }).await,
            DialogResponse::Taken
        );
        assert!(matches!(rx.await.unwrap(), Answer::Cancelled));
    }

    #[tokio::test]
    async fn answering_a_print_that_gave_up_says_so_rather_than_pretending() {
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let rx = shared.lock().await.register(request("p3"));
        shared.lock().await.forget("p3");
        assert_eq!(
            handle(&shared, DialogRequest::Cancel { id: "p3".into() }).await,
            DialogResponse::Unknown
        );
        assert!(rx.await.is_err(), "the waiting print was released");
    }

    #[tokio::test]
    async fn an_id_nobody_registered_is_unknown() {
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        assert_eq!(
            handle(
                &shared,
                DialogRequest::Submit {
                    id: "nope".into(),
                    settings: choice()
                }
            )
            .await,
            DialogResponse::Unknown
        );
    }

    #[tokio::test]
    async fn documents_are_shown_in_the_order_they_arrived() {
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let _a = shared.lock().await.register(request("first"));
        let _b = shared.lock().await.register(request("second"));
        assert_eq!(shared.lock().await.waiting(), 2);
        let got = |r: DialogResponse| match r {
            DialogResponse::Pending { request: Some(r) } => r.id,
            other => panic!("{other:?}"),
        };
        assert_eq!(got(handle(&shared, DialogRequest::Poll).await), "first");
        assert_eq!(got(handle(&shared, DialogRequest::Poll).await), "second");
    }

    #[tokio::test]
    async fn a_poll_over_the_socket_answers_the_shell() {
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let _rx = shared.lock().await.register(request("p4"));
        let (mut client, server) = UnixStream::pair().unwrap();
        tokio::spawn({
            let shared = Arc::clone(&shared);
            async move { serve_connection(&shared, server).await }
        });
        let body = serde_json::to_vec(&DialogRequest::Poll).unwrap();
        client
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        client.write_all(&body).await.unwrap();
        let mut len = [0u8; 4];
        client.read_exact(&mut len).await.unwrap();
        let mut out = vec![0u8; u32::from_be_bytes(len) as usize];
        client.read_exact(&mut out).await.unwrap();
        let answer: DialogResponse = serde_json::from_slice(&out).unwrap();
        assert!(matches!(answer, DialogResponse::Pending { request: Some(r) } if r.id == "p4"));
    }

    #[tokio::test]
    async fn a_parked_ask_is_answered_when_a_document_arrives() {
        // The wake signal. Without it the shell has no reason to look, and a
        // print waits out its timeout in front of somebody who was never told.
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let (mut client, server) = UnixStream::pair().unwrap();
        tokio::spawn({
            let shared = Arc::clone(&shared);
            async move { serve_connection(&shared, server).await }
        });

        let body = serde_json::to_vec(&DialogRequest::Await).unwrap();
        client
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        client.write_all(&body).await.unwrap();

        // Nothing is waiting yet, so the connection is parked. Register one.
        tokio::task::yield_now().await;
        let _rx = shared.lock().await.register(request("late"));

        let mut len = [0u8; 4];
        client.read_exact(&mut len).await.unwrap();
        let mut out = vec![0u8; u32::from_be_bytes(len) as usize];
        client.read_exact(&mut out).await.unwrap();
        let answer: DialogResponse = serde_json::from_slice(&out).unwrap();
        assert!(matches!(answer, DialogResponse::Pending { request: Some(r) } if r.id == "late"));
    }

    #[tokio::test]
    async fn a_parked_ask_takes_what_is_already_there() {
        // A shell reconnecting mid-print must not wait for the NEXT one.
        let shared: Shared = Arc::new(Mutex::new(PendingPrints::default()));
        let _rx = shared.lock().await.register(request("already"));
        let (mut client, server) = UnixStream::pair().unwrap();
        tokio::spawn({
            let shared = Arc::clone(&shared);
            async move { serve_connection(&shared, server).await }
        });
        let body = serde_json::to_vec(&DialogRequest::Await).unwrap();
        client.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
        client.write_all(&body).await.unwrap();
        let mut len = [0u8; 4];
        client.read_exact(&mut len).await.unwrap();
        let mut out = vec![0u8; u32::from_be_bytes(len) as usize];
        client.read_exact(&mut out).await.unwrap();
        let answer: DialogResponse = serde_json::from_slice(&out).unwrap();
        assert!(matches!(answer, DialogResponse::Pending { request: Some(r) } if r.id == "already"));
    }

    #[test]
    fn an_uncompressed_pdf_counts_its_pages() {
        let doc = b"%PDF-1.4\n1 0 obj<</Type /Pages /Kids[2 0 R 3 0 R]>>endobj\n\
                    2 0 obj<</Type /Page>>endobj\n3 0 obj<</Type/Page>>endobj\n";
        assert_eq!(page_count(doc), 2, "the tree node is not a page");
    }

    #[test]
    fn a_document_it_cannot_read_says_it_does_not_know() {
        assert_eq!(page_count(b"%PDF-1.7\n<compressed object streams>"), 0);
        assert_eq!(page_count(b""), 0);
    }
}
