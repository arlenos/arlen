//! Outbound apply: write the foreign-toolkit theme files (GTK, Qt,
//! terminals) from a resolved [`ArlenTheme`], so "one action themes
//! everywhere" reaches non-Svelte apps (GAP-19).
//!
//! The colour generators in [`crate::gtk`], [`crate::qt`] and
//! [`crate::terminal`] are pure `&ArlenTheme -> String` functions; this
//! module is the call site + the file write. Every value they emit is an
//! inert colour or numeric token (proven by the resolve-gate property test
//! in `lib.rs`, which runs each generator over adversarial input), so the
//! written files cannot carry a break-out — that proof is what lets these
//! generators emit into real config paths.
//!
//! Activation by toolkit:
//! - **GTK 3 / 4**: `gtk.css` at the per-version config dir *is* the user
//!   override GTK reads directly, so the write activates the theme with no
//!   further step. Because the filename is fixed by GTK, the write is
//!   guarded: it overwrites only a file Arlen itself generated (one
//!   carrying the marker header) or a missing file, never a hand-authored
//!   `gtk.css` — a foreign file is reported skipped, not clobbered.
//! - **Qt (qt6ct/qt5ct)**: the colour scheme is written to an Arlen-named
//!   file under `colors/`. Selecting it (pointing `qt6ct.conf` at the
//!   scheme + `custom_palette=true`) is a follow-up; the scheme file itself
//!   is the generator's job and lands here.
//! - **Terminals (alacritty/kitty/foot/Xresources)**: written to
//!   Arlen-named colour files. The user's main config must `import` /
//!   `include` them (or `xrdb -merge` for X) to take effect — a follow-up;
//!   writing the colour file is this module's job.
//!
//! All writes are best-effort and independent: one failure is recorded in
//! the [`ApplyReport`] and the rest still run.

use std::path::{Path, PathBuf};

use crate::ArlenTheme;

/// The marker header that tags an Arlen-generated `gtk.css`. The guarded
/// write overwrites a file only when it is absent or starts with this
/// marker, so a user's own `gtk.css` is never clobbered.
const GTK_MARKER: &str = "/* arlen-generated theme";

/// Header prepended to a generated `gtk.css` (carries [`GTK_MARKER`]).
const GTK_HEADER: &str =
    "/* arlen-generated theme (managed by Arlen; edits are overwritten on a theme change) */\n";

/// What an apply run wrote, skipped, or failed. Best-effort: a per-file
/// error never aborts the others.
#[derive(Debug, Default, Clone)]
pub struct ApplyReport {
    /// Files written (created or overwritten).
    pub written: Vec<PathBuf>,
    /// Files left untouched because a foreign (non-Arlen) file already
    /// occupies a fixed-name path (only `gtk.css`).
    pub skipped_foreign: Vec<PathBuf>,
    /// Per-file write errors (path + message).
    pub errors: Vec<(PathBuf, String)>,
}

impl ApplyReport {
    /// Whether every attempted write succeeded (no errors). Skips are not
    /// errors: a skipped foreign `gtk.css` is a deliberate safety outcome.
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Generate and write every foreign-toolkit theme file under `config_dir`.
///
/// `config_dir` is the user config root (`$XDG_CONFIG_HOME`, normally
/// `~/.config`): the per-toolkit files land at `config_dir/gtk-3.0/gtk.css`,
/// `config_dir/qt6ct/colors/arlen.conf`, etc., and the X resources colour
/// file under `config_dir/arlen/`. Returns an [`ApplyReport`] of what was
/// written, skipped, or failed.
pub fn write_foreign_toolkit_configs(theme: &ArlenTheme, config_dir: &Path) -> ApplyReport {
    let mut report = ApplyReport::default();
    let config = config_dir;

    // GTK 3 + 4: gtk.css is the direct override file (fixed name), guarded
    // against clobbering a foreign file. The libadwaita/adw-gtk3
    // named-colour block is identical for both versions.
    let gtk_css = format!("{GTK_HEADER}{}", crate::gtk::generate_gtk_css(theme));
    write_guarded_gtk(&config.join("gtk-3.0/gtk.css"), &gtk_css, &mut report);
    write_guarded_gtk(&config.join("gtk-4.0/gtk.css"), &gtk_css, &mut report);

    // Qt: the colour scheme, Arlen-named, for qt6ct and qt5ct.
    let qt_conf = crate::qt::generate_qt_conf(theme);
    write_owned(&config.join("qt6ct/colors/arlen.conf"), &qt_conf, &mut report);
    write_owned(&config.join("qt5ct/colors/arlen.conf"), &qt_conf, &mut report);

    // Terminals: Arlen-named colour files the user's config imports.
    write_owned(
        &config.join("alacritty/arlen-colors.toml"),
        &crate::terminal::generate_alacritty_toml(theme),
        &mut report,
    );
    write_owned(
        &config.join("kitty/arlen-colors.conf"),
        &crate::terminal::generate_kitty_conf(theme),
        &mut report,
    );
    write_owned(
        &config.join("foot/arlen-colors.ini"),
        &crate::terminal::generate_foot_ini(theme),
        &mut report,
    );
    write_owned(
        &config.join("arlen/colors.Xresources"),
        &crate::terminal::generate_xresources(theme),
        &mut report,
    );

    report
}

/// Write a fixed-name `gtk.css`, but never over a foreign file: write only
/// when the path is absent or already an Arlen-generated file (marker
/// header). A foreign file is recorded in `skipped_foreign`.
fn write_guarded_gtk(path: &Path, content: &str, report: &mut ApplyReport) {
    match std::fs::read_to_string(path) {
        Ok(existing) if !existing.starts_with(GTK_MARKER) => {
            report.skipped_foreign.push(path.to_path_buf());
            return;
        }
        // Absent (Err) or an Arlen-generated file: safe to (over)write.
        _ => {}
    }
    write_file(path, content, report);
}

/// Write an Arlen-named file we own outright (overwrite freely).
fn write_owned(path: &Path, content: &str, report: &mut ApplyReport) {
    write_file(path, content, report);
}

fn write_file(path: &Path, content: &str, report: &mut ApplyReport) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            report.errors.push((path.to_path_buf(), e.to_string()));
            return;
        }
    }
    match std::fs::write(path, content) {
        Ok(()) => report.written.push(path.to_path_buf()),
        Err(e) => report.errors.push((path.to_path_buf(), e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArlenTheme, DARK_TOML};

    fn theme() -> ArlenTheme {
        ArlenTheme::from_bundled(DARK_TOML).expect("bundled dark resolves")
    }

    #[test]
    fn fresh_home_writes_every_toolkit_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        assert!(report.is_clean(), "errors: {:?}", report.errors);
        assert!(report.skipped_foreign.is_empty());
        let c = tmp.path();
        for rel in [
            "gtk-3.0/gtk.css",
            "gtk-4.0/gtk.css",
            "qt6ct/colors/arlen.conf",
            "qt5ct/colors/arlen.conf",
            "alacritty/arlen-colors.toml",
            "kitty/arlen-colors.conf",
            "foot/arlen-colors.ini",
            "arlen/colors.Xresources",
        ] {
            assert!(c.join(rel).is_file(), "missing {rel}");
        }
        // The GTK file is the direct override (carries the marker) and the
        // libadwaita named-colour block.
        let gtk = std::fs::read_to_string(c.join("gtk-4.0/gtk.css")).unwrap();
        assert!(gtk.starts_with(GTK_MARKER));
        assert!(gtk.contains("@define-color"));
    }

    #[test]
    fn reapply_overwrites_an_arlen_owned_gtk_css() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_foreign_toolkit_configs(&theme(), tmp.path());
        // Second apply must overwrite the Arlen-generated file, not skip it.
        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        let gtk3 = tmp.path().join("gtk-3.0/gtk.css");
        assert!(report.written.contains(&gtk3));
        assert!(report.skipped_foreign.is_empty());
    }

    #[test]
    fn a_foreign_gtk_css_is_not_clobbered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let gtk3 = tmp.path().join("gtk-3.0/gtk.css");
        std::fs::create_dir_all(gtk3.parent().unwrap()).unwrap();
        let user_css = "/* my hand-written theme */\nwindow { color: red; }\n";
        std::fs::write(&gtk3, user_css).unwrap();

        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        assert!(report.skipped_foreign.contains(&gtk3));
        // The user's file is untouched.
        assert_eq!(std::fs::read_to_string(&gtk3).unwrap(), user_css);
        // The non-fixed-name files (and the gtk-4.0 path, which was absent)
        // still wrote.
        assert!(report.written.contains(&tmp.path().join("gtk-4.0/gtk.css")));
        assert!(report
            .written
            .contains(&tmp.path().join("kitty/arlen-colors.conf")));
    }
}
