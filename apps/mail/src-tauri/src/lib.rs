// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The mail window: one message file, and only what can be shown honestly.
//!
//! **WHY THERE IS NO HTML HERE, AND IT IS NOT A GAP TO FILL LATER.**
//! `mail-app.md` section 3 makes this an architectural constraint rather than a
//! hardening preference. Two separate reasons, and the second is the one that
//! outlives any sandbox work:
//!
//! 1. A Tauri app on Linux gets no WebKitGTK sandbox by default - `wry` never
//!    calls `webkit_web_context_set_sandbox_enabled`. This tree forces it on
//!    through `WEBKIT_FORCE_SANDBOX` in `main`, so the web process IS contained.
//! 2. That is not the property that matters. EFAIL's finding is that mail
//!    exfiltration is a client-architecture problem, not a crypto or a
//!    process-isolation one: the documented backchannels are CSS `@import`,
//!    `<object data="ftp://">`, headers, attachment preview and certificate
//!    traffic, and a perfectly contained web process phones out through every
//!    one of them. Containing the renderer stops it corrupting the app; it does
//!    not stop the message calling home.
//!
//! So this window shows the headers, the text part, and what the message says
//! about itself - and it says out loud that the HTML part exists and is not
//! being shown, which is a fact about the message rather than a missing feature.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// A message as the window needs it.
/// One part the message carries, as the message describes it.
///
/// The name is the sender's string and nothing here opens or saves the part, so
/// it is shown as written. Whoever adds a save button treats it as a suggestion
/// rather than a destination.
#[derive(serde::Serialize)]
pub struct AttachmentView {
    name: Option<String>,
    media_type: Option<String>,
    bytes: usize,
}

/// A calendar part the message carries, named and not read.
///
/// The core deliberately does not parse the payload (its section-4 note says
/// why), so this carries what the message CLAIMS about the part and nothing
/// derived from its contents.
#[derive(serde::Serialize)]
pub struct InvitationView {
    /// The `method` parameter, lowercased, or `None` when the part named none.
    method: Option<String>,
    bytes: usize,
    /// The part's filename, when it had one, so the window can tell that this
    /// and a row in `attachments` are the same part rather than two things.
    filename: Option<String>,
}

#[derive(Serialize)]
pub struct MessageDto {
    /// The sender as written. Unverified: a display name is whatever the sender
    /// typed, and the surface has to present it as a claim.
    from: Option<String>,
    /// The subject as written.
    subject: Option<String>,
    /// The date line as written.
    date: Option<String>,
    /// The plain-text body.
    text: Option<String>,
    /// Whether a real `text/html` part exists. Its content never leaves the core.
    to: Vec<String>,
    cc: Vec<String>,
    has_html: bool,
    /// The words that appear in only one of the two parts, when both exist.
    /// Data rather than a sentence: the window writes the sentence in the
    /// reader's language around these.
    only_in_text: Vec<String>,
    only_in_html: Vec<String>,
    /// Why the message was refused, when its own headers contradict each other.
    refusal: Option<String>,
    /// Headers that are a way out of this machine.
    channels: Vec<String>,
    /// What the message carries, named and measured, never opened.
    attachments: Vec<AttachmentView>,
    /// Which seal is on the message, as `pgp`, `smime` or `unknown`, when there
    /// is one. A word rather than a sentence: the window says it in the reader's
    /// language, and nothing here decrypts anything.
    sealed: Option<&'static str>,
    /// The calendar part, when there is one. The window says it is there and
    /// opens it in the calendar; nothing here reads it.
    invitation: Option<InvitationView>,
    /// The file this came from, for the surface to name.
    path: String,
}

/// The `.eml` the window was opened on, when it was opened on one.
struct LaunchFile(Option<String>);

/// The file the app was launched with, for the page to ask about on mount.
#[tauri::command]
fn launch_file(state: tauri::State<'_, LaunchFile>) -> Option<String> {
    state.0.clone()
}

/// Read one message file.
///
/// # Errors
/// When the file cannot be read, or holds nothing that parses as a message.
/// Why a message did not open, as a word rather than a sentence.
///
/// There are two causes and both are ordinary for an app that is handed a path
/// by the file manager: the file would not read, or it is not a message. Both
/// used to reach the window as English text built here, inside a frame the
/// catalogue had already translated - so a German reader got half a sentence in
/// each language. `why` survives on the first because the filesystem's own words
/// are the only detail there is.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum ReadProblem {
    Unreadable { why: String },
    NotAMessage,
}

/// Where this machine keeps its mail.
///
/// `$HOME/Maildir`, the convention, because there is no account backend to ask
/// and no config that names one yet - §1 leaves the IMAP client layer an open
/// choice and §5 keeps it open. When one lands it will say where the store is
/// and this becomes its fallback rather than its answer.
///
/// The override is debug-gated, like `install-helper`'s and the greeter's: a
/// release build reading a mailbox path out of the environment it was started in
/// is the same hazard as reading anything else out of it, and the tests are the
/// only caller that needs one.
fn maildir_root() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("ARLEN_MAILDIR") {
        return Some(PathBuf::from(dir));
    }
    dirs::home_dir().map(|h| h.join("Maildir"))
}

/// The mailbox path, as a person would write it.
///
/// The window needs it for the one sentence it cannot compose alone: an empty
/// mailbox is only actionable if it names the place that was empty, and where
/// this machine keeps mail is the host's knowledge. Home-relative, because
/// `~/Maildir` is what a person types and what the convention is called.
#[tauri::command]
fn mail_store() -> Option<String> {
    let root = maildir_root()?;
    Some(home_relative(&root, dirs::home_dir().as_deref()))
}

/// `~/Maildir` for a path inside this user's home, the full path otherwise.
fn home_relative(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// One folder in the rail, as the surface declares it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDto {
    /// `inbox` for the maildir root, else the folder's own directory name.
    ///
    /// A name rather than the empty string the root has on disk: the surface
    /// hands this straight back as `folderId`, and an empty id in a URL or a
    /// log is the kind of value that reads as absent when it is meant.
    id: String,
    kind: &'static str,
    unread: usize,
}

/// One list row. camelCase, because the surface's `Envelope` is - unlike its
/// `Message`, which is the snake_case shape `mail_read` already emits. Two
/// casings on one wire is not tidy, and matching what each consumer declares
/// beats making them agree with each other.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeDto {
    id: String,
    folder_id: String,
    /// The sender's ADDRESS, or empty when the message carried none.
    ///
    /// Empty rather than a sentence: an English "unknown sender" written here
    /// would reach a German reader in English, which is the mistake this app
    /// already made once with its refusal text. What an absent sender looks like
    /// is the catalogue's decision.
    from: String,
    subject: String,
    snippet: String,
    date_ms: i64,
    unread: bool,
}

fn kind_name(k: arlen_mail_core::maildir::FolderKind) -> &'static str {
    use arlen_mail_core::maildir::FolderKind as K;
    match k {
        K::Inbox => "inbox",
        K::Sent => "sent",
        K::Drafts => "drafts",
        K::Archive => "archive",
        K::Trash => "trash",
    }
}

/// The folders in this machine's mailbox.
///
/// An empty list is the honest answer for a machine with no maildir, which is
/// most of them, and it means exactly that one thing: `folders` answers with the
/// inbox for any real maildir, so nothing but an absent store empties it. The
/// surface names the place with `mail_store` rather than inventing an account.
#[tauri::command]
fn mail_folders() -> Vec<FolderDto> {
    let Some(root) = maildir_root() else { return Vec::new() };
    arlen_mail_core::maildir::folders(&root)
        .into_iter()
        .map(|f| FolderDto {
            id: if f.rel.is_empty() { "inbox".to_string() } else { f.rel.clone() },
            kind: kind_name(f.kind),
            unread: f.unread,
        })
        .collect()
}

/// The rows for one folder, newest first.
///
/// A folder id the mailbox does not have yields an empty list rather than an
/// error: the surface asks for whatever it was last showing, and a mailbox that
/// changed under it is an ordinary thing rather than a fault to report.
#[tauri::command]
fn mail_list(folder_id: String) -> Vec<EnvelopeDto> {
    let Some(root) = maildir_root() else { return Vec::new() };
    let rel = if folder_id == "inbox" { String::new() } else { folder_id.clone() };
    // Resolved on its own rather than found in the full listing: the surface asks
    // for one folder's rows at a time, so listing every folder here counted the
    // unread in all of them to learn the name of one.
    let Some(folder) = arlen_mail_core::maildir::folder_at(&root, &rel) else {
        return Vec::new();
    };
    arlen_mail_core::maildir::envelopes(&root, &folder)
        .into_iter()
        .map(|r| EnvelopeDto {
            id: r.id,
            folder_id: folder_id.clone(),
            from: r.from.unwrap_or_default(),
            subject: r.subject.unwrap_or_default(),
            snippet: r.snippet,
            date_ms: r.date_ms.unwrap_or(0),
            unread: r.unread,
        })
        .collect()
}

/// Open one message from the mailbox, by the id its row carried.
///
/// The same `MessageDto` `mail_read` returns - one shape on the wire, as the
/// store asks. The id is resolved through `maildir::message_path`, which refuses
/// what would leave the mailbox and refuses again after following symlinks; a
/// refused id is `NotAMessage`, because there is no id a person can type and a
/// bad one deserves no explanation of the layout.
#[tauri::command]
fn mail_open(id: String) -> Result<MessageDto, ReadProblem> {
    let root = maildir_root().ok_or(ReadProblem::NotAMessage)?;
    let path = arlen_mail_core::maildir::message_path(&root, &id).ok_or(ReadProblem::NotAMessage)?;
    mail_read(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn mail_read(path: String) -> Result<MessageDto, ReadProblem> {
    let raw = std::fs::read(&path).map_err(|e| ReadProblem::Unreadable { why: e.to_string() })?;
    let m = arlen_mail_core::message::read(&raw).map_err(|_| ReadProblem::NotAMessage)?;
    Ok(MessageDto {
        from: m.from,
        subject: m.subject,
        date: m.date,
        text: m.text,
        to: m.to,
        cc: m.cc,
        has_html: m.has_html,
        only_in_text: m.only_in_text,
        only_in_html: m.only_in_html,
        refusal: m.refusal,
        channels: m.channels,
        attachments: m
            .attachments
            .into_iter()
            .map(|a| AttachmentView {
                name: a.name,
                media_type: a.media_type,
                bytes: a.bytes,
            })
            .collect(),
        sealed: m.sealed.map(|s| match s {
            arlen_mail_core::message::Sealed::Pgp => "pgp",
            arlen_mail_core::message::Sealed::Smime => "smime",
            arlen_mail_core::message::Sealed::Unknown => "unknown",
        }),
        invitation: m.invitation.map(|i| InvitationView {
            method: i.method,
            bytes: i.bytes,
            filename: i.filename,
        }),
        path,
    })
}

/// Start the window.
///
/// # Panics
/// When Tauri cannot build the app, which is a broken installation.

/// Why an attachment did not get saved, as a word rather than a sentence.
///
/// Same rule as [`ReadProblem`]: the window owns the wording, this owns the
/// cause. `why` survives only where the filesystem's own words are the detail.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum SaveProblem {
    Unreadable { why: String },
    NotAMessage,
    NoSuchAttachment,
    NoFolder,
    NotWritten { why: String },
}

/// Save one attachment out of a message file, and answer with where it went.
///
/// THE SENDER'S FILENAME IS A SUGGESTION, NOT A DESTINATION - the rule
/// `arlen_mail_core::message::Attachment::name` states and the reason this
/// command takes only the index. Everything but the final component is dropped,
/// so `../../.ssh/authorized_keys` saves as `authorized_keys` into the downloads
/// folder and reaches nothing else. A name that is empty, `.` or `..` after that
/// is not a name, and the part is saved under its position instead.
///
/// The name is otherwise kept AS WRITTEN, including a second extension. Renaming
/// `invoice.pdf.exe` to something calmer would be this app lying about what
/// arrived; what actually keeps it from running is that nothing here marks a
/// saved file executable, and a desktop that launches by extension is a
/// different bug in a different place.
///
/// # Errors
/// When the message will not read, carries no such attachment, there is no
/// downloads folder, or the write fails.
#[tauri::command]
fn mail_save_attachment(path: String, index: usize) -> Result<String, SaveProblem> {
    let raw = std::fs::read(&path).map_err(|e| SaveProblem::Unreadable { why: e.to_string() })?;
    let message =
        arlen_mail_core::message::read(&raw).map_err(|_| SaveProblem::NotAMessage)?;
    let named = message
        .attachments
        .get(index)
        .ok_or(SaveProblem::NoSuchAttachment)?
        .name
        .clone();
    let bytes = arlen_mail_core::message::attachment_bytes(&raw, index)
        .ok_or(SaveProblem::NoSuchAttachment)?;

    let folder = dirs::download_dir().ok_or(SaveProblem::NoFolder)?;
    std::fs::create_dir_all(&folder).map_err(|e| SaveProblem::NotWritten { why: e.to_string() })?;
    let target = free_path(&folder, &safe_name(named.as_deref(), index));
    std::fs::write(&target, &bytes).map_err(|e| SaveProblem::NotWritten { why: e.to_string() })?;
    Ok(target.to_string_lossy().into_owned())
}

/// The sender's suggestion reduced to a single filename, or a positional one.
fn safe_name(named: Option<&str>, index: usize) -> String {
    let from_sender = named
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .and_then(|n| {
            std::path::Path::new(n)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
        })
        .filter(|n| n != "." && n != ".." && !n.is_empty());
    from_sender.unwrap_or_else(|| format!("attachment-{}", index + 1))
}

/// `folder/name`, or the first `name (2)`, `name (3)` that is free.
///
/// NEVER OVERWRITES. Two messages carrying `scan.pdf` are two files, and a
/// second save that silently replaced the first would lose a document while
/// reporting success.
fn free_path(folder: &std::path::Path, name: &str) -> PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let path = std::path::Path::new(name);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy()));
    for n in 2..1000 {
        let candidate = folder.join(format!("{stem} ({n}){}", ext.as_deref().unwrap_or("")));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}


pub fn run() {
    // `arlen-mail <file>`, or the desktop entry's `%f`. Nothing else is read
    // from argv: an app that takes flags from whatever launched it is an app
    // whose behaviour is decided by its caller.
    let launched = std::env::args().nth(1).map(PathBuf::from);
    let launched = launched.and_then(|p| p.canonicalize().ok()).map(|p| p.display().to_string());

    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(LaunchFile(launched))
        .invoke_handler(tauri::generate_handler![
            launch_file,
            mail_read,
            mail_save_attachment,
            mail_folders,
            mail_store,
            mail_list,
            mail_open
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-mail");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_senders_path_becomes_one_filename() {
        // The oldest trick with an attachment name, and the reason this command
        // takes an index rather than a name.
        assert_eq!(safe_name(Some("../../.ssh/authorized_keys"), 0), "authorized_keys");
        assert_eq!(safe_name(Some("/etc/passwd"), 0), "passwd");
        assert_eq!(safe_name(Some("scan.pdf"), 0), "scan.pdf");
    }

    #[test]
    fn a_name_that_is_not_a_name_becomes_a_position() {
        for empty in [None, Some(""), Some("   "), Some("."), Some("..")] {
            assert_eq!(safe_name(empty, 1), "attachment-2", "{empty:?}");
        }
    }

    #[test]
    fn a_second_extension_is_kept_because_renaming_it_would_be_a_lie() {
        // What keeps it from running is that nothing here marks a file
        // executable - not a rename that hides what arrived.
        assert_eq!(safe_name(Some("invoice.pdf.exe"), 0), "invoice.pdf.exe");
    }

    #[test]
    fn a_second_file_of_the_same_name_does_not_replace_the_first() {
        let dir = std::env::temp_dir().join(format!("arlen-mail-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = free_path(&dir, "scan.pdf");
        assert_eq!(first.file_name().unwrap(), "scan.pdf");
        std::fs::write(&first, b"one").unwrap();
        let second = free_path(&dir, "scan.pdf");
        assert_eq!(second.file_name().unwrap(), "scan (2).pdf");
        std::fs::write(&second, b"two").unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"one", "the first must still be there");
        let third = free_path(&dir, "scan.pdf");
        assert_eq!(third.file_name().unwrap(), "scan (3).pdf");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_mailbox_path_reads_the_way_a_person_writes_it() {
        let home = Path::new("/home/ada");
        assert_eq!(home_relative(Path::new("/home/ada/Maildir"), Some(home)), "~/Maildir");
        // A store outside this user's home is named in full: shortening it to a
        // tilde it does not live under would name the wrong place.
        assert_eq!(home_relative(Path::new("/srv/mail"), Some(home)), "/srv/mail");
        assert_eq!(home_relative(Path::new("/home/ada"), Some(home)), "~");
        assert_eq!(home_relative(Path::new("/home/ada/Maildir"), None), "/home/ada/Maildir");
    }

    #[test]
    fn a_name_with_no_extension_still_numbers() {
        let dir = std::env::temp_dir().join(format!("arlen-mail-noext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes"), b"x").unwrap();
        assert_eq!(free_path(&dir, "notes").file_name().unwrap(), "notes (2)");
        std::fs::remove_dir_all(&dir).ok();
    }
}
