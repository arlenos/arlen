//! Putting the resolved Arlen palette inside a bottle (`wine-theming-plan.md`).
//!
//! `sdk/theme`'s Wine spoke turns the resolved theme into a `.reg` document;
//! this is the half that gets it into a prefix. Three things are worth saying
//! about the shape, because each is a decision rather than plumbing.
//!
//! **The document is generated here, never accepted from a caller.** A `.reg`
//! is arbitrary registry content, and a bottle is a confinement whose whole
//! point is that what reaches it is decided by the broker. So the socket verb
//! carries a bottle id and nothing else, and the daemon resolves the person's
//! own theme itself.
//!
//! **The import runs through the same confinement as a Windows program.**
//! `launch_argv` is the one place that assembles a Wine invocation, and
//! `create.rs` already sets the precedent for a non-app one (`wineboot -u`
//! through the same path). So this needs no new grant, no raw spawn and no
//! second copy of the bwrap spec: the sandbox that holds a Windows program
//! holds `regedit` too.
//!
//! **It is applied before a launch, not while one runs.** A running Win32 app
//! does not repaint a `Control Panel\Colors` change - the remedy would be a
//! `WM_SETTINGCHANGE` broadcast nobody has shown can be triggered from Linux -
//! so the reliable model is apply-then-start. What a prefix update does to an
//! applied palette was the open question, and it is answered: `wineboot -u`
//! leaves it alone, measured against a real prefix, so the import does not have
//! to be repeated after one.

use std::io;
use std::path::{Path, PathBuf};

use crate::bottle::Bottle;
use crate::launch::{launch_argv, LaunchError};

/// The document's name inside the bottle's own `C:` drive.
///
/// It lives in the prefix rather than a host temp dir because the confined Wine
/// can only see the prefix: a path outside it is not merely awkward, it is not
/// there at all from inside the sandbox.
pub const REG_FILE: &str = "arlen-theme.reg";

/// Where the document sits on the host.
pub fn reg_host_path(prefix_root: &Path) -> PathBuf {
    prefix_root.join("drive_c").join(REG_FILE)
}

/// The same file as the confined Wine addresses it.
pub fn reg_windows_path() -> String {
    format!("C:\\{REG_FILE}")
}

/// Write the document into the bottle.
///
/// The prefix must have been booted first: `drive_c` is Wine's, not ours, and
/// creating it here would leave a directory that looks like a prefix and is not.
pub fn write_document(prefix_root: &Path, document: &str) -> io::Result<PathBuf> {
    let path = reg_host_path(prefix_root);
    let Some(dir) = path.parent() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no drive_c"));
    };
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a booted prefix", prefix_root.display()),
        ));
    }
    std::fs::write(&path, document)?;
    Ok(path)
}

/// Whether this bottle still needs the document imported.
///
/// The palette is applied before a launch, and doing that unconditionally would
/// put a Wine invocation in front of every single start of every Windows
/// program - seconds, every time, for a registry write that is almost always
/// already there. So the document left in the prefix is the record of what was
/// last imported: identical means the bottle is already wearing this theme, and
/// anything else - a different theme, a changed accent, no file at all because
/// the bottle predates this - means import.
///
/// It compares the document rather than a timestamp or a version, because those
/// are proxies that go wrong in the direction that matters: a theme edited back
/// to what it was bumps a timestamp, and a bottle restored from a backup keeps
/// an old file with a new stamp.
pub fn needs_import(prefix_root: &Path, document: &str) -> bool {
    match std::fs::read_to_string(reg_host_path(prefix_root)) {
        Ok(on_disk) => on_disk != document,
        Err(_) => true,
    }
}

/// The `bwrap` argument list that imports the document, mirroring `boot_argv`.
///
/// No display: an import draws nothing, and a bottle has none to draw on at the
/// moment this runs.
pub fn theme_argv(
    bottle: &Bottle,
    usr: &Path,
    runtime_dir: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Result<Vec<String>, LaunchError> {
    launch_argv(
        bottle,
        usr,
        runtime_dir,
        None,
        &["regedit".to_string(), "/S".to_string(), reg_windows_path()],
        exists,
    )
}

/// How far the theme reached in one bottle.
///
/// The surface promises "best-effort", and a promise like that is only honest
/// if the thing making it can say what it did not manage. Each field is
/// something this code knows rather than something it hopes: nothing here is
/// inferred from the absence of an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeOutcome {
    /// The bottle this is about.
    pub id: String,
    /// The palette, the metrics and the light/dark hint went in.
    pub imported: bool,
    /// Why not, when they did not. Absent on success.
    pub refused: Option<String>,
    /// Whether the UI font FACE is in the prefix.
    ///
    /// Always false today, and that is a fact rather than a gap in the report:
    /// the document points the legacy substitutes at Arlen's UI font, but a
    /// substitute naming a face the prefix does not have leaves Wine on its
    /// fallback. Registering the face - copying the `.ttf` into
    /// `drive_c/windows/Fonts` and adding the `Fonts` record - is a separate
    /// step, so until it exists the surface should say the text did not change.
    pub font_registered: bool,
}

impl ThemeOutcome {
    /// A bottle that took what this code can currently give.
    pub fn imported(id: &str) -> Self {
        Self { id: id.to_string(), imported: true, refused: None, font_registered: false }
    }

    /// A bottle that did not, and what stopped it.
    pub fn refused(id: &str, why: impl std::fmt::Display) -> Self {
        Self {
            id: id.to_string(),
            imported: false,
            refused: Some(why.to_string()),
            font_registered: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottle::{Bottle, Egress};

    fn bottle(prefix: &Path) -> Bottle {
        Bottle {
            id: "b1".to_string(),
            prefix_root: prefix.to_path_buf(),
            grants: Vec::new(),
            egress: Egress::None,
            plumbing: Default::default(),
            program: Vec::new(),
        }
    }

    #[test]
    fn the_document_is_addressed_from_inside_the_bottle() {
        // The host path and the Windows path must name the same file, or the
        // import reads nothing and reports success at doing it.
        let prefix = Path::new("/somewhere/pfx");
        assert_eq!(reg_host_path(prefix), prefix.join("drive_c").join(REG_FILE));
        assert_eq!(reg_windows_path(), format!("C:\\{REG_FILE}"));
    }

    #[test]
    fn an_unbooted_prefix_is_refused_rather_than_given_a_drive_c() {
        // Creating `drive_c` here would leave a directory that looks like a
        // prefix and is not, and the next thing to open it would say so much
        // less clearly than this does.
        let dir = tempfile::tempdir().expect("temp");
        let err = write_document(dir.path(), "REGEDIT4\r\n").expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(format!("{err}").contains("not a booted prefix"));
        assert!(!dir.path().join("drive_c").exists(), "nothing was created");
    }

    #[test]
    fn a_booted_prefix_takes_the_document() {
        let dir = tempfile::tempdir().expect("temp");
        std::fs::create_dir_all(dir.path().join("drive_c")).expect("drive_c");
        let path = write_document(dir.path(), "REGEDIT4\r\n").expect("written");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "REGEDIT4\r\n");
    }

    #[test]
    fn a_bottle_already_wearing_this_theme_is_not_themed_again() {
        let dir = tempfile::tempdir().expect("temp");
        std::fs::create_dir_all(dir.path().join("drive_c")).expect("drive_c");
        let doc = "REGEDIT4\r\n[HKEY_CURRENT_USER\\Control Panel\\Colors]\r\n";

        // No file yet: a bottle made before any of this existed.
        assert!(needs_import(dir.path(), doc));

        write_document(dir.path(), doc).expect("written");
        assert!(!needs_import(dir.path(), doc), "the same theme is already on");

        // A different one - a changed accent is a different document.
        let other = doc.replace("Colors", "Colours");
        assert!(needs_import(dir.path(), &other));
    }

    #[test]
    fn the_import_runs_confined_and_addresses_the_windows_path() {
        let dir = tempfile::tempdir().expect("temp");
        let b = bottle(dir.path());
        let argv = theme_argv(&b, Path::new("/usr"), Path::new("/run/user/1000"), |_| true)
            .expect("argv");
        // Same shape as any launch: bwrap flags, then `--`, then wine.
        let sep = argv.iter().position(|a| a == "--").expect("a separator");
        assert_eq!(
            &argv[sep + 1..],
            ["/usr/bin/wine", "regedit", "/S", &reg_windows_path()]
        );
        assert!(argv[..sep].iter().any(|a| a == "--ro-bind" || a == "--bind"), "it is confined");
    }

    #[test]
    fn a_machine_without_wine_refuses_before_it_assembles_anything() {
        let dir = tempfile::tempdir().expect("temp");
        let b = bottle(dir.path());
        let err = theme_argv(&b, Path::new("/usr"), Path::new("/run/user/1000"), |p| {
            p != Path::new(crate::launch::WINE)
        })
        .expect_err("no wine, no import");
        assert!(format!("{err}").contains("/usr/bin/wine"));
    }

    #[test]
    fn the_outcome_never_claims_a_font_it_did_not_register() {
        // The surface says best-effort, and the only honest version of that is
        // a report that names what did not happen.
        assert!(!ThemeOutcome::imported("b1").font_registered);
        let no = ThemeOutcome::refused("b1", "prefix is not booted");
        assert!(!no.imported);
        assert_eq!(no.refused.as_deref(), Some("prefix is not booted"));
    }
}
