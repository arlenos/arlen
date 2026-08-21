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
#[tauri::command]
fn mail_read(path: String) -> Result<MessageDto, String> {
    let raw = std::fs::read(&path).map_err(|e| format!("could not read {path}: {e}"))?;
    let m = arlen_mail_core::message::read(&raw)?;
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
        path,
    })
}

/// Start the window.
///
/// # Panics
/// When Tauri cannot build the app, which is a broken installation.
pub fn run() {
    // `arlen-mail <file>`, or the desktop entry's `%f`. Nothing else is read
    // from argv: an app that takes flags from whatever launched it is an app
    // whose behaviour is decided by its caller.
    let launched = std::env::args().nth(1).map(PathBuf::from);
    let launched = launched.and_then(|p| p.canonicalize().ok()).map(|p| p.display().to_string());

    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(LaunchFile(launched))
        .invoke_handler(tauri::generate_handler![launch_file, mail_read])
        .run(tauri::generate_context!())
        .expect("error while running arlen-mail");
}
