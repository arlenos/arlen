// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// Whether a failure message is machinery talking rather than something for a
/// person.
///
/// A backend's own words are worth showing - "no such printer", "permission
/// denied on /etc/foo" tells someone what is wrong and sometimes what to do. A
/// JavaScript runtime error names an internal and offers nothing: the reader
/// learns that something called `window.__TAURI_INTERNALS__` is undefined, which
/// is true, unactionable and alarming.
///
/// THIS IS THE SHARED HOME, and it took three copies to get here. `FileBrowser`
/// learned the rule first, by greeting a user with "TypeError: undefined is not
/// an object (evaluating 'window.__TAURI_INTERNALS__.invoke')" in the middle of
/// the pane. The viewers app copied it and wrote "if a third app needs it, that
/// is the moment it moves somewhere both can reach". Settings became the third
/// on 16 August and copied it again, with a note saying the shared home was this
/// file and the move was another lane's to make. It is this lane's now.
///
/// PASS `String(e)`, NOT `e.message`, and the difference is load-bearing. WebKit
/// - which is what Tauri renders with on Linux - words a null dereference as
/// `null is not an object (evaluating 'x.y')`, and Chromium as
/// `Cannot read properties of undefined (reading 'invoke')`. Neither phrase is in
/// the pattern below; both are caught only by the `TypeError` prefix, which
/// `String(e)` keeps and `e.message` drops. The tree has nine places using the
/// `e instanceof Error ? e.message : String(e)` idiom, so the wrong input is one
/// copy-paste away and this is the note that says so.
///
/// Deliberately NARROW. It matches the shapes a missing or broken bridge
/// produces and lets everything else through, because a backend that says why is
/// more useful than a generic sentence. Widening it to hold the two phrases above
/// directly was declined while there were three copies, on the grounds that they
/// would then disagree about what to suppress; with one copy that argument is
/// gone, so a widening is now a single decision rather than three.
export function readsAsInternal(message: string): boolean {
  return /\b(TypeError|ReferenceError|SyntaxError)\b|undefined is not|is not a function|window\.__/.test(
    message,
  );
}
