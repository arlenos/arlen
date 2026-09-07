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

/// Whether a prefix has the font family the substitutes point at.
///
/// **The plan's font step turned out to be unnecessary here, and this is what
/// replaced it.** It described copying the `.ttf` into `drive_c/windows/Fonts`
/// and adding the `Fonts` record, the way winetricks does. Booting a prefix and
/// looking shows Wine has already done it: it registers **2682** faces by
/// absolute `Z:\usr\share\fonts\...` path at boot, Arlen's UI font among
/// them, and it still does so with fontconfig's rules taken away. A bottle
/// binds `/usr` read-only, so those paths resolve from inside the sandbox too.
/// Copying a file that is already reachable would be work whose only visible
/// effect is a second copy of it.
///
/// So the honest thing is not to register the font but to ASK, because the
/// answer is a fact about this machine rather than about our code: a font Arlen
/// ships would be there and one a person names in a customization file might not
/// be.
///
/// It reads the registry file rather than running `reg query`, because this runs
/// while deciding what to report and a Wine invocation per report is a cost the
/// surface does not need. The match is on the value NAME, which Wine writes as
/// `Family Style (TrueType)` - so `Inter` is present when a name begins with it.
pub fn font_available(prefix_root: &Path, family: &str) -> bool {
    if family.is_empty() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(prefix_root.join("system.reg")) else {
        return false;
    };
    let needle = family.to_lowercase();
    let mut in_fonts = false;
    for line in text.lines() {
        if line.starts_with('[') {
            in_fonts = line
                .to_lowercase()
                .contains("currentversion\\fonts]");
            continue;
        }
        if !in_fonts {
            continue;
        }
        // `"Inter Bold (TrueType)"="Z:\\usr\\..."`. The face name is the part
        // in quotes, and a family is present when a face name starts with it.
        if let Some(name) = line.strip_prefix('"').and_then(|r| r.split('"').next()) {
            let lower = name.to_lowercase();
            if lower == needle || lower.starts_with(&format!("{needle} ")) {
                return true;
            }
        }
    }
    false
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

    /// The shape Wine actually writes, copied from a booted prefix rather than
    /// imagined: a `Software\...\Fonts` section whose value names are face
    /// names and whose values are `Z:` paths into the host's font directories.
    const REAL_SHAPE: &str = concat!(
        "WINE REGISTRY Version 2\n\n",
        "[Software\\Microsoft\\Windows NT\\CurrentVersion\\FontSubstitutes] 1788\n",
        "\"Inter\"=\"Something Else\"\n\n",
        "[Software\\Microsoft\\Windows\\CurrentVersion\\Fonts] 1788\n",
        "\"Inter Bold (TrueType)\"=\"Z:\\\\usr\\\\share\\\\fonts\\\\inter\\\\Inter.ttc\"\n",
        "\"Noto Sans (TrueType)\"=\"Z:\\\\usr\\\\share\\\\fonts\\\\noto\\\\NotoSans-Regular.ttf\"\n\n",
        "[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall] 1788\n",
        "\"Cantarell Reader\"=\"nothing to do with fonts\"\n",
    );

    #[test]
    fn the_font_question_is_asked_of_the_prefix_and_answered_from_the_fonts_key() {
        let dir = tempfile::tempdir().expect("temp");
        std::fs::write(dir.path().join("system.reg"), REAL_SHAPE).expect("write");

        // Present as a face under the family name.
        assert!(font_available(dir.path(), "Inter"));
        assert!(font_available(dir.path(), "Noto Sans"));
        // Not there at all.
        assert!(!font_available(dir.path(), "Comic Sans MS"));
        // A name that appears in the file but NOT in the fonts section. Reading
        // the whole file for the string would say yes to this, which is how a
        // report ends up describing a font that is an uninstall entry.
        assert!(!font_available(dir.path(), "Cantarell"));
        // The substitutes section names the family too, on the LEFT of an `=`,
        // and it is a request rather than a fact.
        assert!(!font_available(dir.path(), "Something Else"));
        // A prefix with no registry at all, and an empty family.
        assert!(!font_available(Path::new("/nowhere"), "Inter"));
        assert!(!font_available(dir.path(), ""));
    }

}
