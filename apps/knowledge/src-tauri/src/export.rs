// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Write the recorded timeline to a file the user owns.
//!
//! The Export item invoked a command nothing registered, so the surface said
//! "Export isn't wired to the graph yet" - honest, and a dead end on the one
//! promise this app makes about the record it keeps: that it is the user's, and
//! they can take it.
//!
//! It writes JSON, not a rendering. What the timeline shows is a phrasing of the
//! captured facts (a verb id the catalogue turns into a word, an object, a
//! moment); the export carries the facts. Somebody reading their own history in
//! six months with a script wants the fields, and a Markdown rendering would have
//! baked in one language and one set of sentences.
//!
//! Where it goes is not a dialog: this app has no file-chooser and adding one to
//! answer "where" would be a portal round-trip for a decision with an obvious
//! answer. It lands in the user's download directory under a dated name, and the
//! command returns the path so the surface can say where it went - an export that
//! reports success without saying where is barely better than one that fails.

use std::path::PathBuf;

use serde::Serialize;

use crate::timeline::TimelineItem;

/// The exported document: the items plus enough about the export itself that a
/// file found later explains where it came from.
#[derive(Debug, Serialize)]
struct Export<'a> {
    /// What wrote it.
    source: &'static str,
    /// The shape of `items`, so a later reader can tell versions apart.
    format: &'static str,
    /// When it was written, Unix seconds.
    exported_at: i64,
    /// The recorded spine, newest first, exactly as the app read it.
    items: &'a [TimelineItem],
}

/// The user's download directory: `XDG_DOWNLOAD_DIR` when set, else
/// `$HOME/Downloads`, else the home directory itself.
///
/// Read from the environment rather than by parsing `user-dirs.dirs`, because
/// the session already exports the XDG user dirs and a second parser is a second
/// thing to disagree with the first.
fn download_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DOWNLOAD_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let downloads = home.join("Downloads");
    Some(if downloads.is_dir() { downloads } else { home })
}

/// `arlen-timeline-YYYY-MM-DD.json`, and `-2`, `-3` … if that name is taken.
///
/// Never overwrites: an export is a copy of the user's own history, and silently
/// replacing yesterday's file with today's would destroy a record they chose to
/// keep.
fn free_path(dir: &std::path::Path, date: &str) -> PathBuf {
    let first = dir.join(format!("arlen-timeline-{date}.json"));
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let p = dir.join(format!("arlen-timeline-{date}-{n}.json"));
        if !p.exists() {
            return p;
        }
    }
    first
}

/// `YYYY-MM-DD` for a Unix timestamp, in UTC.
///
/// Hand-rolled because this crate carries no date library and the alternative is
/// a dependency for one filename. Civil-from-days, the standard algorithm.
fn ymd(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Write the timeline to the user's downloads and return the path.
#[tauri::command]
pub async fn knowledge_timeline_export() -> Result<String, String> {
    let items = crate::timeline::knowledge_timeline().await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let doc = Export {
        source: "arlen-knowledge",
        format: "timeline-items-v1",
        exported_at: now,
        items: &items,
    };
    let json = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    let dir = download_dir().ok_or_else(|| "no home directory to export into".to_string())?;
    let path = free_path(&dir, &ymd(now));
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_is_the_calendar_date_not_an_offset() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_786_233_600), "2026-08-09");
        // A leap day, which the naive divide-by-365 gets wrong.
        assert_eq!(ymd(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn an_existing_export_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("arlen-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = free_path(&dir, "2026-08-09");
        assert!(first.ends_with("arlen-timeline-2026-08-09.json"));
        std::fs::write(&first, b"{}").unwrap();
        let second = free_path(&dir, "2026-08-09");
        assert!(second.ends_with("arlen-timeline-2026-08-09-2.json"), "{second:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
