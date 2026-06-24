//! Default-handler registration for the viewer (`quickview-plan.md`: register as
//! the default MIME handler for the image + audio formats, so the system opens
//! them in the viewer, not the browser).
//!
//! Pure generation of the two install artifacts - the viewer's `.desktop` entry
//! (with its `MimeType=` list) and the `mimeapps.list` `[Default Applications]`
//! block - from the format set the detection core owns ([`IMAGE_MIMES`] /
//! [`AUDIO_MIMES`]), so the registered handlers and the decodable formats stay
//! in step. Writing the files at install is the packaging step on top.

use arlen_viewers_core::{AUDIO_MIMES, IMAGE_MIMES};
use std::path::Path;

/// The viewer's desktop-file id (the value referenced in `mimeapps.list`).
pub const DESKTOP_FILE: &str = "org.arlen.Viewer.desktop";

/// The viewer's `.desktop` entry, registering it as a handler for `mimes`.
/// `exec` is the launcher command; `%f` passes the opened file. `NoDisplay=true`
/// keeps it out of the app menu - it is a file handler the FM launches, not a
/// directly-launched app.
pub fn desktop_entry(exec: &str, mimes: &[&str]) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Arlen Viewer\n\
         Comment=View images and play audio\n\
         Exec={exec} %f\n\
         Terminal=false\n\
         NoDisplay=true\n\
         Categories=Graphics;Viewer;AudioVideo;\n\
         MimeType={};\n",
        mimes.join(";"),
    )
}

/// The `mimeapps.list` `[Default Applications]` block making the viewer the
/// default handler for each MIME in `mimes`.
pub fn default_associations(mimes: &[&str]) -> String {
    let mut s = String::from("[Default Applications]\n");
    for m in mimes {
        s.push_str(m);
        s.push('=');
        s.push_str(DESKTOP_FILE);
        s.push('\n');
    }
    s
}

/// Every MIME the viewer registers for: images (decodable now) + audio (handled
/// by the player). The packaging step passes this to [`desktop_entry`] +
/// [`default_associations`].
pub fn all_mimes() -> Vec<&'static str> {
    IMAGE_MIMES.iter().chain(AUDIO_MIMES.iter()).copied().collect()
}

/// Set the viewer as the default handler for each of `mimes` inside an existing
/// `mimeapps.list` body, preserving every other association and section.
///
/// Each MIME is set to [`DESKTOP_FILE`] under `[Default Applications]` (creating
/// that section if absent): an existing default for one of our MIMEs is
/// overwritten in place, a MIME not yet present is appended to the section, and
/// every other app's default + every other section (`[Added Associations]`,
/// comments) is kept verbatim. Idempotent - merging an already-merged body is a
/// no-op. This is the non-clobbering merge a runtime/install registration needs;
/// it never rewrites the user's whole handler map.
pub fn merge_default_associations(existing: &str, mimes: &[&str]) -> String {
    const HEADER: &str = "[Default Applications]";
    let entry = |m: &str| format!("{m}={DESKTOP_FILE}");
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();

    match lines.iter().position(|l| l.trim() == HEADER) {
        Some(h) => {
            // The section body runs to the next `[section]` header or EOF.
            let end = lines[h + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map_or(lines.len(), |p| h + 1 + p);
            let mut missing = Vec::new();
            for &m in mimes {
                let key = format!("{m}=");
                match lines[h + 1..end].iter_mut().find(|l| l.starts_with(&key)) {
                    Some(line) => *line = entry(m), // overwrite a prior default
                    None => missing.push(m),
                }
            }
            // Append the not-yet-present MIMEs at the end of the section.
            for (i, m) in missing.iter().enumerate() {
                lines.insert(end + i, entry(m));
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
            lines.push(HEADER.to_string());
            lines.extend(mimes.iter().map(|&m| entry(m)));
        }
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}

/// Register the viewer as the system default handler for its image + audio
/// MIMEs: write its `.desktop` entry into `apps_dir` and merge its defaults into
/// `mimeapps_path` (preserving the user's other associations via
/// [`merge_default_associations`]). `exec` is the launcher command. The parent
/// directories are created as needed. Idempotent.
pub fn register_default_handler(apps_dir: &Path, mimeapps_path: &Path, exec: &str) -> std::io::Result<()> {
    let mimes = all_mimes();
    std::fs::create_dir_all(apps_dir)?;
    std::fs::write(apps_dir.join(DESKTOP_FILE), desktop_entry(exec, &mimes))?;

    let existing = std::fs::read_to_string(mimeapps_path).unwrap_or_default();
    let merged = merge_default_associations(&existing, &mimes);
    if let Some(parent) = mimeapps_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(mimeapps_path, merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_entry_lists_the_mime_types_and_exec() {
        let entry = desktop_entry("/usr/bin/arlen-viewer", IMAGE_MIMES);
        assert!(entry.contains("Exec=/usr/bin/arlen-viewer %f"));
        assert!(entry.contains("MimeType=image/png;image/jpeg;"), "the MimeType list is present");
        assert!(entry.trim_end().ends_with(';'), "the MimeType list is semicolon-terminated");
        assert!(entry.contains("NoDisplay=true"));
    }

    #[test]
    fn default_associations_map_each_mime_to_the_viewer() {
        let block = default_associations(IMAGE_MIMES);
        assert!(block.starts_with("[Default Applications]\n"));
        assert!(block.contains(&format!("image/png={DESKTOP_FILE}\n")));
        assert!(block.contains(&format!("image/heic={DESKTOP_FILE}\n")));
        // One line per MIME plus the header.
        assert_eq!(block.lines().count(), IMAGE_MIMES.len() + 1);
    }

    #[test]
    fn all_mimes_covers_images_and_audio() {
        let all = all_mimes();
        assert!(all.contains(&"image/png"));
        assert!(all.contains(&"audio/flac"));
        assert_eq!(all.len(), IMAGE_MIMES.len() + AUDIO_MIMES.len());
    }

    #[test]
    fn merge_into_an_empty_list_creates_the_section() {
        let out = merge_default_associations("", &["image/png", "audio/flac"]);
        assert!(out.contains("[Default Applications]\n"));
        assert!(out.contains(&format!("image/png={DESKTOP_FILE}\n")));
        assert!(out.contains(&format!("audio/flac={DESKTOP_FILE}\n")));
    }

    #[test]
    fn merge_preserves_foreign_associations_and_other_sections() {
        let existing = "[Default Applications]\n\
                        application/pdf=org.pdf.Reader.desktop\n\
                        \n\
                        [Added Associations]\n\
                        text/plain=org.text.Editor.desktop;\n";
        let out = merge_default_associations(existing, &["image/png"]);
        // Our MIME is added under Default Applications...
        assert!(out.contains(&format!("image/png={DESKTOP_FILE}")));
        // ...the foreign default is untouched...
        assert!(out.contains("application/pdf=org.pdf.Reader.desktop"));
        // ...and the other section + its entry survive verbatim.
        assert!(out.contains("[Added Associations]"));
        assert!(out.contains("text/plain=org.text.Editor.desktop;"));
    }

    #[test]
    fn merge_overwrites_an_existing_default_for_our_mime() {
        let existing = "[Default Applications]\nimage/png=org.gnome.eog.desktop\n";
        let out = merge_default_associations(existing, &["image/png"]);
        assert!(out.contains(&format!("image/png={DESKTOP_FILE}")));
        assert!(!out.contains("org.gnome.eog.desktop"), "the prior default is replaced");
        // The MIME is not duplicated.
        assert_eq!(out.matches("image/png=").count(), 1);
    }

    #[test]
    fn merge_is_idempotent() {
        let mimes = ["image/png", "image/jpeg", "audio/flac"];
        let once = merge_default_associations("application/pdf=x.desktop\n", &mimes);
        let twice = merge_default_associations(&once, &mimes);
        assert_eq!(once, twice);
    }

    #[test]
    fn register_writes_the_desktop_entry_and_merges_the_list() {
        let dir = std::env::temp_dir().join(format!("arlen-mimeapps-test-{}", std::process::id()));
        let apps = dir.join("applications");
        let list = dir.join("mimeapps.list");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&list, "[Default Applications]\napplication/pdf=org.pdf.desktop\n").unwrap();

        register_default_handler(&apps, &list, "/usr/bin/arlen-viewer").expect("register");

        let entry = std::fs::read_to_string(apps.join(DESKTOP_FILE)).unwrap();
        assert!(entry.contains("Exec=/usr/bin/arlen-viewer %f"));
        let merged = std::fs::read_to_string(&list).unwrap();
        assert!(merged.contains(&format!("image/png={DESKTOP_FILE}")));
        assert!(merged.contains("application/pdf=org.pdf.desktop"), "user's default kept");
        std::fs::remove_dir_all(&dir).ok();
    }
}
