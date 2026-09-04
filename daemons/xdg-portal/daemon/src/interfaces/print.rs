//! The `org.freedesktop.impl.portal.Print` backend (printing-plan.md PRN-R2).
//!
//! Bridges a portal print request to the built `arlen-print` service (CUPS,
//! PRN-R1). This module holds the pure mapping from the portal's print settings
//! (`a{sv}`, the GTK/CUPS vocabulary) to `arlen_print` [`JobOptions`]; the
//! interface impl (`PreparePrint` / `Print` + document-fd handling, the Request
//! pattern) builds on it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use arlen_print::backend::{ColorMode, Duplex, JobOptions, PrintBackend, PrintError, PrintSubmission};
use arlen_print::cups::CupsBackend;
use arlen_print::service::PrintService;
use audit_proto::sink::LedgerAuditSink;
use xdg_portal_arlen_protocol::print as wire;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::print_ipc::{self, Answer};

/// How long a print waits for somebody to answer its dialog.
///
/// A person reads the document name, picks a printer and thinks about it, so this
/// is minutes rather than seconds. When it runs out the request is forgotten and
/// the app is told the print was cancelled, which is the safe end of that
/// failure: a timeout must never mean "print it anyway".
const DIALOG_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// The maximum document the Print backend reads from a caller's fd (512 MiB). A
/// larger document is refused rather than read unbounded into memory, so a
/// misbehaving app (or a `/dev/zero` fd) cannot OOM the portal daemon.
const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;

// THE APP'S OWN SETTINGS MAP IS NO LONGER READ, and the mapping that read it is
// gone rather than kept warm. It turned the caller's `a{sv}` into job options,
// which is precisely the behaviour §2 rules out: the app chose and the person
// was never asked. What the app sends now only ever influences what it renders,
// through the settings this backend hands BACK from the dialog's answer. If the
// dialog ever wants to open with the app's suggestion preselected, that is a
// field on the request rather than a second path into the printer.

/// What a `PreparePrint` staged, keyed by the token it returned; a subsequent
/// `Print` recalls it by that token (one-shot).
///
/// It holds THE PERSON'S CHOICE, not the app's settings, and that is the change
/// that makes this backend match the plan. It used to stage whatever the calling
/// app sent, so the app chose the printer and the person never saw a dialog -
/// which is the isolation property §2 is about, the same one the file picker has.
#[derive(Default)]
struct Prepared {
    next_token: u32,
    by_token: HashMap<u32, wire::PrintChoice>,
}

/// The `org.freedesktop.impl.portal.Print` backend state: the arlen-print service
/// over the CUPS print system (recording submits to the audit ledger - the printer
/// and destination, never the document) plus the `PreparePrint` -> `Print`
/// token staging.
pub struct Print {
    service: PrintService<CupsBackend>,
    prepared: Mutex<Prepared>,
    /// The documents waiting for somebody to choose how to print them.
    dialog: print_ipc::Shared,
    /// Where the request ids come from. Only ever compared, never parsed.
    next_id: Mutex<u64>,
}

impl Print {
    /// Construct the backend over CUPS + the default audit ledger socket, asking
    /// the dialog behind `dialog` before anything is printed.
    pub fn new(dialog: print_ipc::Shared) -> Self {
        Self {
            service: PrintService::new(
                CupsBackend::default(),
                Arc::new(LedgerAuditSink::at_default_socket()),
            ),
            prepared: Mutex::new(Prepared::default()),
            dialog,
            next_id: Mutex::new(0),
        }
    }

    /// Stage a choice and return the token that recalls it.
    fn stage(&self, choice: wire::PrintChoice) -> u32 {
        let mut p = self.prepared.lock().unwrap();
        p.next_token = p.next_token.wrapping_add(1).max(1);
        let token = p.next_token;
        p.by_token.insert(token, choice);
        token
    }

    /// Recall (and remove) a staged choice by its token, if present.
    fn take(&self, token: u32) -> Option<wire::PrintChoice> {
        self.prepared.lock().unwrap().by_token.remove(&token)
    }

    /// A fresh request id.
    fn mint_id(&self) -> String {
        let mut n = self.next_id.lock().unwrap();
        *n = n.wrapping_add(1);
        format!("print-{n}")
    }

    /// Put a document in front of somebody and wait for their answer.
    ///
    /// A dialog that never answers - no shell running, a shell that died with the
    /// window open - times out into a cancel. Nothing is printed on a timeout,
    /// ever: an app that asked to print and got silence has lost nothing, while
    /// an app that asked and got an unattended print has put a document on paper
    /// nobody chose to put there.
    async fn ask(&self, app_id: &str, title: &str, page_count: u32) -> Answer {
        let id = self.mint_id();
        let request = wire::PrintRequest {
            id: id.clone(),
            title: title.to_string(),
            app_id: app_id.to_string(),
            app_name: app_id.to_string(),
            page_count,
        };
        let rx = self.dialog.lock().await.register(request);
        match tokio::time::timeout(DIALOG_WAIT, rx).await {
            Ok(Ok(answer)) => answer,
            _ => {
                self.dialog.lock().await.forget(&id);
                tracing::info!(app_id, id, "portal Print: nobody answered the dialog");
                Answer::Cancelled
            }
        }
    }
}

/// Map the dialog's answer to job options.
///
/// The printer is the person's, so it is carried separately; everything here is
/// what goes on the job. The range travels as the typed text - `arlen_print`
/// owns what `1-5, 8` means, and a second reading of it here is how the two
/// would come to disagree.
pub fn job_options_from_choice(choice: &wire::PrintChoice) -> JobOptions {
    JobOptions {
        copies: Some(choice.copies).filter(|&n| n >= 1),
        duplex: Some(match choice.duplex {
            wire::Duplex::OneSided => Duplex::OneSided,
            wire::Duplex::TwoSidedLong => Duplex::TwoSidedLongEdge,
            wire::Duplex::TwoSidedShort => Duplex::TwoSidedShortEdge,
        }),
        color: Some(match choice.color {
            wire::Color::Color => ColorMode::Color,
            wire::Color::Mono => ColorMode::Monochrome,
        }),
        media: media_keyword(&choice.paper),
        page_ranges: match choice.range_mode {
            wire::RangeMode::Range => Some(choice.range_text.clone()),
            // "All" is the absence of a range, and "current page" is a thing only
            // the app knows the number of - it renders the page it means and
            // sends that, so from here it is the whole of what arrived.
            wire::RangeMode::All | wire::RangeMode::Current => None,
        },
    }
}

/// The IPP media keyword for the dialog's paper name. An unknown name leaves the
/// printer's own default rather than guessing a size.
pub fn media_keyword(paper: &str) -> Option<String> {
    match paper {
        "a4" => Some("iso_a4_210x297mm".to_string()),
        "letter" => Some("na_letter_8.5x11in".to_string()),
        "legal" => Some("na_legal_8.5x14in".to_string()),
        _ => None,
    }
}

/// The portal settings map that carries the person's choice back to the app.
///
/// `PreparePrint` answers with settings the app then renders against, so the
/// paper size and duplex have to travel in the vocabulary the app speaks (the
/// GTK/CUPS one), not the dialog's. The range goes back too: an app that knows
/// only pages 2 to 4 are wanted can render only those.
pub fn settings_from_choice(choice: &wire::PrintChoice) -> HashMap<String, OwnedValue> {
    let mut out = HashMap::new();
    let mut put = |key: &str, value: String| {
        if let Ok(v) = OwnedValue::try_from(Value::from(value)) {
            out.insert(key.to_string(), v);
        }
    };
    put("n-copies", choice.copies.max(1).to_string());
    put(
        "sides",
        match choice.duplex {
            wire::Duplex::OneSided => "one-sided",
            wire::Duplex::TwoSidedLong => "two-sided-long-edge",
            wire::Duplex::TwoSidedShort => "two-sided-short-edge",
        }
        .to_string(),
    );
    put(
        "print-color-mode",
        match choice.color {
            wire::Color::Color => "color",
            wire::Color::Mono => "monochrome",
        }
        .to_string(),
    );
    if let Some(media) = media_keyword(&choice.paper) {
        put("media", media);
    }
    if matches!(choice.range_mode, wire::RangeMode::Range) && !choice.range_text.trim().is_empty() {
        put("page-ranges", choice.range_text.clone());
    }
    out
}

/// Submit a document to the printer the person chose, with their options. Fails
/// closed rather than silently dropping the job. Generic over the backend so it
/// is testable with a mock.
async fn submit_document<B: PrintBackend>(
    service: &PrintService<B>,
    app_id: &str,
    document: &[u8],
    title: Option<&str>,
    choice: &wire::PrintChoice,
) -> Result<i32, PrintError> {
    if choice.printer.trim().is_empty() {
        return Err(PrintError::Invalid("no printer was chosen".to_string()));
    }
    let options = job_options_from_choice(choice);
    // A range the dialog let through that this cannot read refuses the job. The
    // option exists to print less, so falling back to everything is the one
    // outcome worse than not printing.
    if let Some(text) = &options.page_ranges {
        if let Err(e) = arlen_print::pages::parse(text, 0) {
            return Err(PrintError::Invalid(e.message()));
        }
    }
    let submission = PrintSubmission {
        printer: &choice.printer,
        document,
        title,
        mime: None,
        options,
    };
    // `app_id` is the calling app: the audit records it as the acting principal.
    service.submit(app_id, &submission).await
}

#[interface(name = "org.freedesktop.impl.portal.Print")]
impl Print {
    /// Interface version.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    /// Stage the print settings and return a token the subsequent `Print` recalls.
    /// The interactive dialog is arlen-ui's; this backend stages the request's own
    /// settings so the print proceeds with the app's chosen options.
    #[allow(clippy::too_many_arguments)]
    async fn prepare_print(
        &self,
        _handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _title: &str,
        settings: HashMap<String, OwnedValue>,
        _page_setup: HashMap<String, OwnedValue>,
        _options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // §2: only the authenticated frontend may reach this impl name.
        if !crate::interfaces::sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await
        {
            tracing::warn!("refusing a PreparePrint call from a sender that is not the portal frontend");
            return (2, HashMap::new());
        }
        // The app's own settings are not what gets used - they are what it would
        // have used if nobody asked. The dialog asks.
        let _ = settings;
        // No document yet on this path, so no page count: `PreparePrint` is the
        // moment BEFORE the app renders. Zero is the dialog's "not known", and
        // the range control stands down rather than offering a range over a
        // length nobody measured.
        let choice = match self.ask(app_id, _title, 0).await {
            Answer::Print(choice) => *choice,
            Answer::Cancelled => {
                tracing::info!(app_id, "portal Print: prepare declined");
                return (1, HashMap::new());
            }
        };
        let results = settings_from_choice(&choice);
        let token = self.stage(choice);
        tracing::info!(app_id, token, "portal Print: prepared");
        let mut results = results;
        if let Ok(v) = OwnedValue::try_from(Value::U32(token)) {
            results.insert("token".to_string(), v);
        }
        (0, results)
    }

    /// Print the document on `fd` using the settings staged under `options["token"]`.
    /// Response `0` = printed, `2` = failed.
    #[allow(clippy::too_many_arguments)]
    async fn print(
        &self,
        _handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        fd: zbus::zvariant::OwnedFd,
        options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // §2: frontend-only; a direct caller could print a handed fd under a
        // forged app_id, bypassing the frontend's authentication.
        if !crate::interfaces::sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await
        {
            tracing::warn!("refusing a Print call from a sender that is not the portal frontend");
            return (2, HashMap::new());
        }
        let staged = options
            .get("token")
            .and_then(|v| u32::try_from(v.clone()).ok())
            .and_then(|t| self.take(t));

        // Read the document off the reactor (a large PDF must not block it),
        // BOUNDED: a caller-supplied fd (a huge file, or `/dev/zero`) must not be
        // read unbounded into memory and OOM the portal daemon.
        let doc = match tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let f = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
            let mut buf = Vec::new();
            // Read one byte past the cap so an over-size document is detected.
            f.take(MAX_DOCUMENT_BYTES.saturating_add(1))
                .read_to_end(&mut buf)
                .map(|_| buf)
        })
        .await
        {
            Ok(Ok(buf)) if buf.len() as u64 <= MAX_DOCUMENT_BYTES => buf,
            Ok(Ok(_)) => {
                tracing::warn!(app_id, "portal Print: document exceeds the size cap; refused");
                return (2, HashMap::new());
            }
            _ => {
                tracing::warn!(app_id, "portal Print: could not read the document fd");
                return (2, HashMap::new());
            }
        };

        // A staged token means somebody already chose in `PreparePrint`; asking
        // again would be a second dialog over one print. Without one, this is the
        // dialog moment, and here the document IS in hand, so the page count is
        // real.
        let choice = match staged {
            Some(choice) => choice,
            None => match self.ask(app_id, title, crate::print_ipc::page_count(&doc)).await {
                Answer::Print(choice) => *choice,
                Answer::Cancelled => {
                    tracing::info!(app_id, "portal Print: declined");
                    return (1, HashMap::new());
                }
            },
        };

        match submit_document(&self.service, app_id, &doc, Some(title), &choice).await {
            Ok(job) => {
                tracing::info!(app_id, job, "portal Print: submitted");
                (0, HashMap::new())
            }
            Err(e) => {
                tracing::warn!(app_id, error = %e, "portal Print: submit failed");
                (2, HashMap::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdg_portal_arlen_protocol::print::{Color, Duplex as WireDuplex, RangeMode};

    fn choice() -> wire::PrintChoice {
        wire::PrintChoice {
            printer: "Office HP".into(),
            copies: 3,
            range_mode: RangeMode::Range,
            range_text: "1-5, 8".into(),
            duplex: WireDuplex::TwoSidedLong,
            color: Color::Mono,
            paper: "a4".into(),
        }
    }

    #[test]
    fn the_choice_becomes_the_job() {
        let o = job_options_from_choice(&choice());
        assert_eq!(o.copies, Some(3));
        assert!(matches!(o.duplex, Some(Duplex::TwoSidedLongEdge)));
        assert!(matches!(o.color, Some(ColorMode::Monochrome)));
        assert_eq!(o.media.as_deref(), Some("iso_a4_210x297mm"));
        assert_eq!(o.page_ranges.as_deref(), Some("1-5, 8"));
    }

    #[test]
    fn printing_everything_carries_no_range() {
        let mut c = choice();
        c.range_mode = RangeMode::All;
        assert_eq!(job_options_from_choice(&c).page_ranges, None);
        // "This page" is a page only the app can identify: it renders the one it
        // means and sends that, so from here it is the whole of what arrived.
        c.range_mode = RangeMode::Current;
        assert_eq!(job_options_from_choice(&c).page_ranges, None);
    }

    #[test]
    fn a_paper_nobody_knows_leaves_the_printers_own_default() {
        assert_eq!(media_keyword("a4").as_deref(), Some("iso_a4_210x297mm"));
        assert_eq!(media_keyword("a3"), None);
    }

    #[test]
    fn the_answer_goes_back_in_the_vocabulary_the_app_speaks() {
        let out = settings_from_choice(&choice());
        let text = |k: &str| match Value::from(out.get(k).unwrap().clone()) {
            Value::Str(s) => s.to_string(),
            other => panic!("{k} is {other:?}"),
        };
        assert_eq!(text("n-copies"), "3");
        assert_eq!(text("sides"), "two-sided-long-edge");
        assert_eq!(text("print-color-mode"), "monochrome");
        assert_eq!(text("media"), "iso_a4_210x297mm");
        assert_eq!(text("page-ranges"), "1-5, 8");
    }

    #[test]
    fn an_empty_range_is_not_sent_back_as_one() {
        let mut c = choice();
        c.range_text = "   ".into();
        assert!(!settings_from_choice(&c).contains_key("page-ranges"));
    }
}
