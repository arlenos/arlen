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

use std::path::PathBuf;

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
        .invoke_handler(tauri::generate_handler![launch_file, mail_read, mail_save_attachment])
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
    fn a_name_with_no_extension_still_numbers() {
        let dir = std::env::temp_dir().join(format!("arlen-mail-noext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes"), b"x").unwrap();
        assert_eq!(free_path(&dir, "notes").file_name().unwrap(), "notes (2)");
        std::fs::remove_dir_all(&dir).ok();
    }
}
