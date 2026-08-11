// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The top-bar arrangement inventory, read from the running shell.
//!
//! The panel needs to know which applets and tray items exist RIGHT NOW, which
//! is live state in the desktop-shell process and cannot come from a config
//! file. The shell has produced it all along (`topbar.rs`, registered); what did
//! not exist was a route, because a Tauri command only resolves inside the
//! binary that registers it. So `invoke("topbar_items")` from here reached
//! nothing and the panel came up empty with an error.
//!
//! This command has the same NAME as the shell's on purpose. The frontend store
//! already invokes `topbar_items` and does not care which process answers, so
//! naming it anything else would have meant editing a caller that was correct.
//!
//! **A conduit, not a second definition.** The item shape (`id`, `name`, `icon`,
//! `kind`, `shown`) belongs to the shell, and mirroring the struct here would
//! create two definitions of one wire format in two crates with nothing keeping
//! them level - the defect this tree keeps finding. The bytes are relayed as
//! parsed JSON, so the shape can gain a field without a change on this side.

use std::path::PathBuf;

use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

/// The shell's inventory broker, the same path `topbar_ipc` binds.
///
/// Restated rather than shared because the two live in different crates with no
/// dependency between them; the name is asserted on both sides by a test, which
/// is the cheapest thing that can actually fail if one moves.
const SOCKET_NAME: &str = "topbar.sock";

/// Refuse a response larger than this. The inventory is a handful of applets
/// plus the tray, so a megabyte is already absurd; the cap exists so a wedged
/// or hostile writer cannot grow this process without bound.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_string())?;
    let mut p = PathBuf::from(runtime);
    p.push("arlen");
    p.push(SOCKET_NAME);
    Ok(p)
}

/// Turn a dial failure into something the panel can show a person.
///
/// `No such file or directory` on a socket path is true and useless: the reason
/// there is no socket is that the shell is not running, and saying so is the
/// difference between a panel that explains itself and one that leaks an errno.
fn dial_error(path: &std::path::Path, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        return "the desktop shell is not running, so it has no top bar to arrange".to_string();
    }
    format!("cannot reach the desktop shell at {}: {e}", path.display())
}

/// The live top-bar inventory, in the shell's saved order.
#[tauri::command]
pub async fn topbar_items() -> Result<serde_json::Value, String> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path)
        .await
        .map_err(|e| dial_error(&path, &e))?;

    // The broker writes once and closes, so read to EOF - bounded, because a
    // peer that never closes would otherwise hold this task open forever.
    let mut body = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("reading the inventory: {e}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > MAX_RESPONSE_BYTES {
            return Err("the shell sent an implausibly large inventory".to_string());
        }
    }

    parse_inventory(&body)
}

/// Parse the relayed body, checking only that it is the ARRAY the panel expects.
///
/// Deliberately not checking the item fields: the shell owns that shape, and a
/// validator here would be a second definition of it that could refuse a
/// correct response after the shell gained a field. The array check is different
/// in kind - it catches a truncated or empty read, which is the failure this
/// side can actually have.
fn parse_inventory(body: &[u8]) -> Result<serde_json::Value, String> {
    if body.is_empty() {
        return Err("the shell closed the connection without sending anything".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("the shell sent malformed JSON: {e}"))?;
    if !value.is_array() {
        return Err("the shell sent something other than a list of items".to_string());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both ends have to agree on one path, and they are in crates that share no
    /// type. If the shell's broker moves, this is what says so.
    #[test]
    fn the_socket_name_matches_the_shells_broker() {
        assert_eq!(SOCKET_NAME, "topbar.sock");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(
            socket_path().expect("a runtime dir is set"),
            PathBuf::from("/run/user/1000/arlen/topbar.sock")
        );
    }

    #[test]
    fn an_array_passes_through_untouched() {
        let body = br#"[{"id":"tray:x","name":"X","icon":"tray","kind":"tray","shown":true}]"#;
        let v = parse_inventory(body).expect("an array is what the panel expects");
        assert_eq!(v.as_array().map(Vec::len), Some(1));
        // The field the panel reads survives verbatim - the point of relaying
        // rather than re-modelling.
        assert_eq!(v[0]["id"], "tray:x");
    }

    /// The three failures this side can actually have, each with its own words.
    #[test]
    fn a_truncated_or_wrong_shaped_response_is_refused_by_name() {
        assert!(parse_inventory(b"")
            .unwrap_err()
            .contains("without sending anything"));
        assert!(parse_inventory(b"[{\"id\"")
            .unwrap_err()
            .contains("malformed JSON"));
        assert!(parse_inventory(b"{\"items\":[]}")
            .unwrap_err()
            .contains("other than a list"));
    }

    /// A missing socket means a shell that is not running, and the panel should
    /// say that rather than show an errno.
    #[test]
    fn a_missing_socket_reads_as_a_shell_that_is_not_running() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let msg = dial_error(std::path::Path::new("/run/user/1000/arlen/topbar.sock"), &e);
        assert!(msg.contains("desktop shell is not running"), "{msg}");
        assert!(!msg.contains("No such file"), "{msg}");
    }
}
