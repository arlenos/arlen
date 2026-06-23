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
}
