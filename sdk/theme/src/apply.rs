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
//! - **Qt (qt6ct/qt5ct)**: the colour scheme goes to an Arlen-named file under
//!   `colors/`, and the `qt6ct.conf` beside it points at the scheme with
//!   `custom_palette=true` - both, because a scheme nothing selects is a file
//!   nobody reads. That conf is guarded like the GTK ones: qt6ct rewrites it
//!   whenever somebody uses its window, and that file is then theirs. It still
//!   needs `QT_QPA_PLATFORMTHEME=qt6ct` in the session, which nothing in Arlen
//!   sets today - our half is done and theirs is named rather than assumed.
//! - **Terminals**: an Arlen-named colour file, and the main config beside it
//!   that reads it - `include` for kitty and foot, `[general] import` for
//!   alacritty - each guarded, so a config the person wrote is theirs and ours
//!   is written only where there was none. The three differ in what kind of path
//!   they accept and each was read out of its own upstream documentation rather
//!   than guessed; `terminal.rs` says which.
//!
//!   **Xresources is NOT written**, and that is a measurement rather than an
//!   omission. It is not a file a program opens on start: it has to be loaded
//!   into the X server's resource database by `xrdb -merge`, and an Arlen
//!   machine has no X server to load it into - the image names no `xwayland`
//!   and no `xrdb` package, and `daemons/session` deliberately UNSETS `DISPLAY`
//!   before starting the compositor so it takes the DRM path. So the file could
//!   not be read even by someone who installed an X client. The generator stays
//!   in `terminal.rs` for the day XWayland is on the image; until then writing
//!   the file would be the same untruth as a surface reporting a write it never
//!   made.
//!
//! - **CLI tools**: colour fragments for git, delta, fzf, starship and a base16
//!   scheme for editors. git and delta are WIRED - a guarded
//!   `$XDG_CONFIG_HOME/git/config` includes both fragments, and git reads it
//!   after `~/.gitconfig` so anything the person set themselves still wins. The
//!   other three cannot be: fzf takes its palette from `FZF_DEFAULT_OPTS`, which
//!   is a shell rc the person owns; starship reads exactly one config file and
//!   has no include; the base16 scheme is for whichever editor plugin they use.
//!   Each of those three carries the activation line in its own first comment,
//!   which is the most a file can do for itself.
//!
//! All writes are best-effort and independent: one failure is recorded in
//! the [`ApplyReport`] and the rest still run.

use std::path::{Path, PathBuf};

use crate::ArlenTheme;

/// The marker header that tags an Arlen-generated `gtk.css`. The guarded
/// write overwrites a file only when it is absent or starts with this
/// marker, so a user's own `gtk.css` is never clobbered.
const GTK_MARKER: &str = "/* arlen-generated theme";

/// The marker that tags an Arlen-generated INI: the GTK settings files and the
/// qt6ct/qt5ct selection. A separate one from [`GTK_MARKER`] because these files
/// are INI, where the CSS comment the sheet carries is not a comment at all -
/// writing the CSS marker into one would give the parser an error on line one.
const INI_MARKER: &str = "# arlen-generated";

/// Where GTK3 looks for installed themes, highest precedence first. The user's
/// two directories then the system's, which is the order GTK itself searches.
fn gtk_theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".themes"));
    }
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("themes"));
    }
    dirs.push(PathBuf::from("/usr/share/themes"));
    dirs
}

/// Where icon themes are looked for, in the freedesktop search order: the two
/// per-user directories then the system's.
fn icon_theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".icons"));
    }
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("icons"));
    }
    dirs.push(PathBuf::from("/usr/share/icons"));
    dirs
}

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
    /// What this run decided the interface should be, once the machine had been
    /// looked at.
    ///
    /// Carried out rather than left inside because `settings.ini` is not the
    /// only reader of these six decisions: measured on 8 September, GTK3 takes
    /// its theme from `org.gnome.desktop.interface` whenever those schemas are
    /// installed and IGNORES the file entirely - and they are on our image, so
    /// the file alone selects nothing. The second writer needs the same answer
    /// rather than its own copy of the detection, which is what this field is.
    pub selection: Option<crate::gtk::InterfaceSelection>,
}

impl ApplyReport {
    /// Whether every attempted write succeeded (no errors). Skips are not
    /// errors: a skipped foreign `gtk.css` is a deliberate safety outcome.
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Whether the theme is actually in place for one toolkit.
///
/// Not "did we write it" - whether what is on disk right now is ours. The two
/// differ on any machine where somebody has written their own config, which the
/// guarded write correctly refuses to overwrite: the write is reported skipped,
/// the theme reaches nothing there, and until this existed the only trace was an
/// `info` line in the shell's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolkitReach {
    /// Every file this toolkit needs is one we wrote.
    Ours,
    /// At least one is somebody else's, so the theme does not reach this toolkit
    /// and will not until they move it. Carries the first such file, because a
    /// surface that can name it saves the reader looking for it.
    Blocked(PathBuf),
    /// None of them are there. The apply has not run, or it failed.
    Absent,
}

/// The files each toolkit's reach depends on, relative to the config root.
///
/// Only the ones a person can have written themselves: the Arlen-named files
/// under `colors/` and the like are ours outright and always overwritten, so
/// they can never block anything and would only add noise here.
const REACH_FILES: [(&str, &[&str]); 4] = [
    ("gtk3", &["gtk-3.0/gtk.css", "gtk-3.0/settings.ini"]),
    ("gtk4", &["gtk-4.0/gtk.css", "gtk-4.0/settings.ini"]),
    ("qt", &["qt6ct/qt6ct.conf"]),
    (
        "terminal",
        &["kitty/kitty.conf", "foot/foot.ini", "alacritty/alacritty.toml"],
    ),
];

/// Whether one file on disk is one we wrote, by its marker.
fn is_ours(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.starts_with(GTK_MARKER) || text.starts_with(INI_MARKER))
}

/// Report, per toolkit, whether the theme is actually in place.
///
/// Read-only and cheap: it opens the handful of files the guarded writes target
/// and looks at the first line. Nothing here writes, so a surface can ask it as
/// often as it likes.
///
/// One foreign file blocks the toolkit even when its sibling is ours, and that
/// is the honest reading rather than a harsh one: a GTK3 app with our colours
/// and somebody else's settings file is not following the theme, it is following
/// half of it, and half is what makes a coverage badge a lie.
pub fn toolkit_reach(config_dir: &Path) -> std::collections::BTreeMap<String, ToolkitReach> {
    REACH_FILES
        .iter()
        .map(|(toolkit, files)| {
            let states: Vec<(PathBuf, Option<bool>)> = files
                .iter()
                .map(|f| {
                    let path = config_dir.join(f);
                    let ours = is_ours(&path);
                    (path, ours)
                })
                .collect();
            let foreign = states.iter().find(|(_, ours)| *ours == Some(false));
            let reach = match foreign {
                Some((path, _)) => ToolkitReach::Blocked(path.clone()),
                None if states.iter().any(|(_, ours)| *ours == Some(true)) => ToolkitReach::Ours,
                None => ToolkitReach::Absent,
            };
            ((*toolkit).to_string(), reach)
        })
        .collect()
}

/// Generate and write every foreign-toolkit theme file under `config_dir`.
///
/// `config_dir` is the user config root (`$XDG_CONFIG_HOME`, normally
/// `~/.config`): the per-toolkit files land at `config_dir/gtk-3.0/gtk.css`,
/// `config_dir/qt6ct/colors/arlen.conf`, etc., and the X resources colour
/// file under `config_dir/arlen/`. Returns an [`ApplyReport`] of what was
/// written, skipped, or failed.
pub fn write_foreign_toolkit_configs(theme: &ArlenTheme, config_dir: &Path) -> ApplyReport {
    // No per-toolkit divergence: every target sees the same resolved theme.
    write_toolkit_configs(theme, theme, theme, theme, &[], config_dir)
}

/// Override-aware apply: resolve each toolkit's theme (the shared theme plus
/// that toolkit's sparse `[override.<toolkit>]`) and write. A theme with no
/// `[override.*]` produces byte-identical output to
/// [`write_foreign_toolkit_configs`], so this is always safe to prefer.
///
/// Takes the source strings (not a resolved theme) because the per-toolkit
/// override is applied during resolution ([`ArlenTheme::resolve_toolkit`]); the
/// CLI colour tools ride the Terminal override (they share the terminal
/// palette), and the sound-name map rides the base theme (sounds are not a
/// rendering toolkit).
pub fn write_foreign_toolkit_configs_with_overrides(
    bundled: &str,
    user_theme: Option<&str>,
    customization: Option<&str>,
    config_dir: &Path,
) -> Result<ApplyReport, crate::ResolveError> {
    use crate::Toolkit;
    let base = ArlenTheme::resolve(bundled, user_theme, customization)?;
    let gtk = ArlenTheme::resolve_toolkit(bundled, user_theme, customization, Toolkit::Gtk)?;
    let qt = ArlenTheme::resolve_toolkit(bundled, user_theme, customization, Toolkit::Qt)?;
    let term = ArlenTheme::resolve_toolkit(bundled, user_theme, customization, Toolkit::Terminal)?;
    Ok(write_toolkit_configs(&base, &gtk, &qt, &term, &[], config_dir))
}

/// One foreign-toolkit spoke the apply writes for, named as the Toolkits page
/// names its rows.
///
/// GTK 3 and GTK 4 are separate here although they share one `[override.gtk]`
/// table, and the asymmetry is the files' rather than a slip: the two versions
/// read different files, so switching one off is expressible, while a theme file
/// has a single GTK override block, so diverging one from the other is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spoke {
    /// GTK 3 apps (`gtk-3.0/`).
    Gtk3,
    /// GTK 4 / libadwaita apps (`gtk-4.0/`).
    Gtk4,
    /// Qt apps (`qt6ct` / `qt5ct`).
    Qt,
    /// Terminal emulators (alacritty / kitty / foot).
    Terminal,
}

impl Spoke {
    /// The page's row id for this spoke.
    pub fn id(self) -> &'static str {
        match self {
            Self::Gtk3 => "gtk3",
            Self::Gtk4 => "gtk4",
            Self::Qt => "qt",
            Self::Terminal => "terminal",
        }
    }

    /// The spoke a row id names, or `None` for a row that is not one of ours
    /// (`arlen` is native and `wine` is not written from here).
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Gtk3, Self::Gtk4, Self::Qt, Self::Terminal]
            .into_iter()
            .find(|s| s.id() == id)
    }
}

/// Remove a file we wrote, leaving anybody else's alone.
///
/// The mirror of [`write_guarded`] and it uses the same evidence: a file whose
/// first line is not our marker was not written by us, so a switch-off must not
/// delete it. An absent file is not an error - turning off a spoke that was
/// never written is a no-op, and should be silent rather than reported.
fn remove_guarded(path: &Path, report: &mut ApplyReport) {
    match std::fs::read_to_string(path) {
        Ok(existing) if existing.starts_with(GTK_MARKER) || existing.starts_with(INI_MARKER) => {
            if let Err(e) = std::fs::remove_file(path) {
                report.errors.push((path.to_path_buf(), e.to_string()));
            }
        }
        // Somebody else's file, or none. Either way, nothing to take away.
        _ => {}
    }
}

/// The files a switched-off spoke gives up.
///
/// **Only the ones that STEER the toolkit**, never the Arlen-named palette files
/// beside them. A person who followed the Toolkits page's own prerequisite line
/// and added `include arlen-colors.conf` to a `kitty.conf` they wrote themselves
/// has a config that breaks if the palette disappears, and breaking somebody's
/// hand-written file is not a thing an off switch may do. So off removes every
/// file of ours that points a toolkit at the theme, and leaves the palette
/// sitting there unread. The consequence is worth stating on the row: an include
/// the person wrote themselves keeps working until they take it out, because it
/// is theirs.
fn steering_files(spoke: Spoke) -> &'static [&'static str] {
    match spoke {
        Spoke::Gtk3 => &["gtk-3.0/gtk.css", "gtk-3.0/settings.ini"],
        Spoke::Gtk4 => &["gtk-4.0/gtk.css", "gtk-4.0/settings.ini"],
        Spoke::Qt => &["qt6ct/qt6ct.conf", "qt5ct/qt5ct.conf"],
        Spoke::Terminal => &["alacritty/alacritty.toml", "kitty/kitty.conf", "foot/foot.ini"],
    }
}

/// Write every foreign-toolkit theme file, each section from its own resolved
/// theme so a per-toolkit override diverges only that target. `base_theme` backs
/// the non-toolkit outputs (the sound-name map).
///
/// Public because the string-based entry point above is not the only caller that
/// needs per-toolkit divergence. The desktop shell resolves a theme through its
/// own chain - `appearance.toml`'s accent and radius-intensity overrides, then
/// the accessibility settings - none of which the resolver here knows about, so
/// it cannot hand this crate the source strings and get the same answer back. It
/// resolves each toolkit itself and calls this. Until it did, `[override.gtk]`
/// was read by the resolver, written by Settings, and reached no file on the
/// machine: the shell's apply used the no-divergence entry point.
pub fn write_toolkit_configs(
    base_theme: &ArlenTheme,
    gtk_theme: &ArlenTheme,
    qt_theme: &ArlenTheme,
    term_theme: &ArlenTheme,
    off: &[Spoke],
    config_dir: &Path,
) -> ApplyReport {
    let mut report = ApplyReport::default();
    let config = config_dir;
    let on = |spoke: Spoke| !off.contains(&spoke);
    // Give back what a switched-off spoke was steering, before writing anything
    // else. Done here rather than at the switch so the state on disk follows
    // from the setting every time the theme is applied, including the run after
    // a machine was off when the switch was flipped.
    for spoke in off {
        for file in steering_files(*spoke) {
            remove_guarded(&config.join(file), &mut report);
        }
    }

    // GTK 3 + 4: gtk.css is the direct override file (fixed name), guarded
    // against clobbering a foreign file. The libadwaita/adw-gtk3
    // named-colour block is identical for both versions.
    // Not one sheet for both: GTK3's parser has no custom properties and
    // answers the `--window-radius` block with an error every app logs at
    // startup, so it gets the colours and GTK4 gets the colours plus the radius.
    if on(Spoke::Gtk3) {
        let gtk3_css = format!("{GTK_HEADER}{}", crate::gtk::generate_gtk3_css(gtk_theme));
        write_guarded(&config.join("gtk-3.0/gtk.css"), &gtk3_css, GTK_MARKER, &mut report);
    }
    if on(Spoke::Gtk4) {
        let gtk4_css = format!("{GTK_HEADER}{}", crate::gtk::generate_gtk_css(gtk_theme));
        write_guarded(&config.join("gtk-4.0/gtk.css"), &gtk4_css, GTK_MARKER, &mut report);
    }

    // GTK 3 settings: which widget theme, icon set, cursor and font a GTK3 app
    // uses. None of that is expressible in the override sheet above. Same guard:
    // a hand-authored settings.ini is left alone, because this one is a file
    // people really do write themselves.
    //
    // NOT THE SELECTOR ON A MACHINE THAT HAS THE GNOME SCHEMAS, and that is
    // measured rather than assumed. On 8 September a GTK3 probe under a correct
    // file of ours resolved `gtk-theme-name='Adwaita'` and none of these values;
    // GTK3 prefers `org.gnome.desktop.interface` whenever it is installed and
    // falls back to the SCHEMA's default rather than to this file. Our image has
    // those schemas, so the shell also names the theme there - see
    // `InterfaceSelection`, which both writers render. This file is what a
    // machine WITHOUT them reads, which is why it is still written.
    let selected = crate::gtk::installed_gtk_theme(&crate::gtk::GTK_THEME_CANDIDATES, &gtk_theme_dirs());
    // The icon set is named only when it is one, which is not the same as the
    // directory being there - see `installed_icon_theme`.
    let icons = crate::gtk::installed_icon_theme(&gtk_theme.icons.theme, &icon_theme_dirs())
        .then_some(gtk_theme.icons.theme.as_str());
    let mut selection = crate::gtk::interface_selection(gtk_theme, selected, icons);
    // Same rule as the icon theme one line up, and measured the same way - at the
    // caller, so the selection itself stays a pure function of the theme. A
    // family fontconfig cannot resolve is not named: the toolkit would substitute
    // silently and every foreign app would disagree with ours about the body
    // font. An absent `fc-match` says nothing, so the font is kept.
    if crate::gtk::font_family_installed(crate::wine::first_family(&gtk_theme.typography.font_sans))
        == Some(false)
    {
        selection.font = None;
    }
    if on(Spoke::Gtk3) {
        let ini = crate::gtk::generate_gtk_settings_ini(&selection);
        // Carried out only when GTK3 is on, because its one consumer beyond this
        // file is the gsettings writer that NAMES our widget theme system-wide.
        // Reporting it for a switched-off spoke would leave the file gone and the
        // theme still selected, which is the loudest half of the two.
        report.selection = Some(selection.clone());
        write_guarded(&config.join("gtk-3.0/settings.ini"), &ini, INI_MARKER, &mut report);
    }

    // GTK 4 reads its OWN settings file and none of GTK 3's, so without this a
    // GTK4 app took our colours and the system's icons, cursor and font. Measured
    // on GTK 4.20: all five keys are honoured from `gtk-4.0/settings.ini`,
    // `gtk-application-prefer-dark-theme` included - which is what makes a GTK4
    // app that never linked libadwaita go dark, the case that rendered light in
    // the 7 September toolkit shot. libadwaita apps take the same answer through
    // the portal instead, and agreeing with ourselves in both channels is the
    // point.
    //
    // Re-measured on 8 September against the schema, because the GTK3 half of
    // this turned out to be a fiction: GTK4 reads this file when the schema has
    // nothing to say, and the SCHEMA WINS when it does. A probe with
    // `font-name='Schema Font 9'` set there reported exactly that, with our
    // file's font untouched on disk. So this is GTK4's fallback rather than its
    // authority, and the authority is written by the shell.
    //
    // No theme name here, and the `None` is deliberate rather than a fallback:
    // Arlen ships no GTK4 widget theme and will not, because there is no stable
    // selector contract to write one against. Naming one would be the claim the
    // GTK3 side omits for the other reason.
    // The same answer minus the theme name, rather than a second derivation of
    // it: the two files must not be able to disagree about the font or the
    // cursor.
    if on(Spoke::Gtk4) {
        let mut gtk4_selection = selection;
        gtk4_selection.gtk_theme = None;
        let gtk4_ini = crate::gtk::generate_gtk_settings_ini(&gtk4_selection);
        write_guarded(&config.join("gtk-4.0/settings.ini"), &gtk4_ini, INI_MARKER, &mut report);
    }

    // Qt: the colour scheme, Arlen-named, for qt6ct and qt5ct.
    if on(Spoke::Qt) {
    let qt_conf = crate::qt::generate_qt_conf(qt_theme);
    write_owned(&config.join("qt6ct/colors/arlen.conf"), &qt_conf, &mut report);
    write_owned(&config.join("qt5ct/colors/arlen.conf"), &qt_conf, &mut report);

    // And the file that SELECTS it. Writing a scheme nothing points at is the
    // same nothing the GTK3 theme was until its settings file existed - qt6ct
    // reads the palette from `color_scheme_path` and only honours it under
    // `custom_palette`, so the scheme above was decoration on its own.
    //
    // Guarded, and this one is the reason the guard exists: qt6ct writes this
    // file itself whenever somebody uses its window, and that file is theirs.
    // Ours is written when there is none - which is the state a fresh machine is
    // in, and the state where the Toolkits page currently asks the PERSON to go
    // and do this by hand.
    //
    // NB it still needs `QT_QPA_PLATFORMTHEME=qt6ct` in the session to take
    // effect, and nothing in Arlen sets that today. So this closes our half and
    // leaves theirs visible rather than pretending the chain is whole.
    for (dir, file) in [("qt6ct", "qt6ct.conf"), ("qt5ct", "qt5ct.conf")] {
        let scheme = config.join(dir).join("colors/arlen.conf");
        let select = crate::qt::generate_qt_select_conf(&scheme.to_string_lossy(), icons);
        write_guarded(&config.join(dir).join(file), &select, INI_MARKER, &mut report);
    }
    }

    // Terminals: Arlen-named colour files the user's config imports.
    if on(Spoke::Terminal) {
    write_owned(
        &config.join("alacritty/arlen-colors.toml"),
        &crate::terminal::generate_alacritty_toml(term_theme),
        &mut report,
    );
    write_guarded(
        &config.join("alacritty/alacritty.toml"),
        &crate::terminal::generate_alacritty_import("arlen-colors.toml"),
        INI_MARKER,
        &mut report,
    );
    write_owned(
        &config.join("kitty/arlen-colors.conf"),
        &crate::terminal::generate_kitty_conf(term_theme),
        &mut report,
    );
    // And the config that includes it. Guarded: a kitty.conf somebody wrote is
    // theirs, and ours is only written where there is none - which is the state
    // a fresh machine is in and the state where the Toolkits page currently asks
    // the PERSON to add an include by hand. Only kitty; `generate_kitty_include`
    // says why the other three are left alone.
    write_guarded(
        &config.join("kitty/kitty.conf"),
        &crate::terminal::generate_kitty_include("arlen-colors.conf"),
        INI_MARKER,
        &mut report,
    );
    write_owned(
        &config.join("foot/arlen-colors.ini"),
        &crate::terminal::generate_foot_ini(term_theme),
        &mut report,
    );
    // foot's include must be absolute, so the resolved path goes in rather than
    // a bare name.
    let foot_colours = config.join("foot/arlen-colors.ini");
    write_guarded(
        &config.join("foot/foot.ini"),
        &crate::terminal::generate_foot_include(&foot_colours.to_string_lossy()),
        INI_MARKER,
        &mut report,
    );
    }

    // The theme's per-event sound-name map, for the Notification Daemon to merge
    // into its sound config (it owns playback; the user's notifications.toml
    // overrides still win). Arlen-owned, overwritten freely.
    write_owned(
        &config.join("arlen/sounds.toml"),
        &crate::sounds::generate_sound_overrides(base_theme),
        &mut report,
    );

    // CLI tools: an Arlen-owned colour file the user's own config sources. fzf
    // reads its palette from `FZF_DEFAULT_OPTS`, so the finder matches the theme.
    write_owned(
        &config.join("arlen/fzf-colors.sh"),
        &crate::cli::generate_fzf_colors(term_theme),
        &mut report,
    );
    write_owned(
        &config.join("arlen/git-colors.gitconfig"),
        &crate::cli::generate_git_colors(term_theme),
        &mut report,
    );
    // starship reads one config file, so this is emitted as an Arlen-owned
    // fragment the user pastes or symlinks rather than a file we own outright.
    write_owned(
        &config.join("arlen/starship-palette.toml"),
        &crate::cli::generate_starship_palette(term_theme),
        &mut report,
    );
    write_owned(
        &config.join("arlen/delta.gitconfig"),
        &crate::cli::generate_delta_config(term_theme),
        &mut report,
    );
    // The git fragments above are inert until something includes them, so the
    // include goes in too - guarded, so a `git/config` the person wrote is
    // theirs. Absolute paths: `[include] path` resolves against the including
    // file, and the fragments live one directory over.
    let git_colors = config.join("arlen/git-colors.gitconfig");
    let git_delta = config.join("arlen/delta.gitconfig");
    write_guarded(
        &config.join("git/config"),
        &crate::cli::generate_git_include(
            &git_colors.to_string_lossy(),
            &git_delta.to_string_lossy(),
        ),
        INI_MARKER,
        &mut report,
    );
    write_owned(
        &config.join("arlen/base16-arlen.yaml"),
        &crate::cli::generate_nvim_base16(term_theme),
        &mut report,
    );

    report
}

/// Write a fixed-name `gtk.css`, but never over a foreign file: write only
/// when the path is absent or already an Arlen-generated file (marker
/// header). A foreign file is recorded in `skipped_foreign`.
fn write_guarded(path: &Path, content: &str, marker: &str, report: &mut ApplyReport) {
    match std::fs::read_to_string(path) {
        Ok(existing) if !existing.starts_with(marker) => {
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
    fn per_toolkit_override_diverges_only_the_gtk_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        // A GTK-only accent override to a distinctive green.
        let custom = r##"
[override.gtk.color.semantic]
accent = "#00ff00"
"##;
        let report =
            write_foreign_toolkit_configs_with_overrides(DARK_TOML, None, Some(custom), tmp.path())
                .expect("apply with overrides");
        assert!(report.is_clean(), "errors: {:?}", report.errors);
        let gtk = std::fs::read_to_string(tmp.path().join("gtk-4.0/gtk.css")).unwrap();
        let qt = std::fs::read_to_string(tmp.path().join("qt6ct/colors/arlen.conf")).unwrap();
        // The GTK stylesheet carries the override accent; the Qt scheme keeps the
        // bundled accent, so the override diverges only the GTK target.
        assert!(
            gtk.contains("00ff00"),
            "gtk.css must carry the GTK-only override accent"
        );
        assert!(
            !qt.contains("00ff00"),
            "qt scheme must keep the base accent, not the GTK override"
        );
    }

    #[test]
    fn overrides_apply_matches_plain_apply_when_no_override() {
        // With no [override.*], the override-aware path writes byte-identical
        // files to the plain path.
        let a = tempfile::TempDir::new().unwrap();
        let b = tempfile::TempDir::new().unwrap();
        write_foreign_toolkit_configs(&theme(), a.path());
        write_foreign_toolkit_configs_with_overrides(DARK_TOML, None, None, b.path())
            .expect("apply");
        let gtk_a = std::fs::read_to_string(a.path().join("gtk-4.0/gtk.css")).unwrap();
        let gtk_b = std::fs::read_to_string(b.path().join("gtk-4.0/gtk.css")).unwrap();
        assert_eq!(gtk_a, gtk_b);
    }

    #[test]
    fn a_font_this_machine_cannot_resolve_is_not_named() {
        // Skips where fontconfig cannot be asked, because an absent tool is not
        // evidence and the apply keeps naming the font in that case.
        let Some(false) = crate::gtk::font_family_installed("Definitely Not A Font 12345") else {
            return;
        };
        let mut theme = theme();
        theme.typography.font_sans = "\"Definitely Not A Font 12345\", sans-serif".into();
        let tmp = tempfile::TempDir::new().unwrap();
        let report = write_foreign_toolkit_configs(&theme, tmp.path());
        assert!(report.is_clean(), "errors: {:?}", report.errors);
        for rel in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
            let ini = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
            assert!(!ini.contains("gtk-font-name"), "{rel} names a font nobody has:\n{ini}");
            // The rest of the file is unaffected.
            assert!(ini.contains("gtk-cursor-theme-name="), "{rel} lost its other keys");
        }
        assert_eq!(report.selection.and_then(|s| s.font), None);
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
            // The notification sound map and the CLI-tool colour files.
            "arlen/sounds.toml",
            "arlen/fzf-colors.sh",
            "arlen/git-colors.gitconfig",
            "arlen/delta.gitconfig",
            "git/config",
            "arlen/starship-palette.toml",
            "arlen/base16-arlen.yaml",
        ] {
            assert!(c.join(rel).is_file(), "missing {rel}");
        }
        // Xresources is deliberately absent: nothing on an Arlen machine can
        // load it. It needs `xrdb -merge` against an X server, and the image
        // names neither `xwayland` nor `xrdb` while the session unsets DISPLAY.
        // If XWayland ever lands, wire the merge FIRST and then restore the
        // write - a file with no reader is what this whole module stopped doing.
        assert!(!c.join("arlen/colors.Xresources").exists());

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

    /// The same guard on the settings file, and it matters more there: a
    /// hand-written `settings.ini` is where people put their font and their
    /// cursor size, and it is a file GTK itself tells them to edit.
    #[test]
    fn a_hand_written_settings_ini_is_not_clobbered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ini = tmp.path().join("gtk-3.0/settings.ini");
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        let mine = "[Settings]\ngtk-font-name=Comic Sans MS 18\n";
        std::fs::write(&ini, mine).unwrap();

        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        assert!(report.skipped_foreign.contains(&ini));
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), mine);
    }

    /// And it does write one when there is nothing to protect, carrying the
    /// marker that lets the next run recognise its own work.
    #[test]
    fn an_absent_settings_ini_is_written_with_the_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ini = tmp.path().join("gtk-3.0/settings.ini");
        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        assert!(report.written.contains(&ini), "{report:?}");
        let written = std::fs::read_to_string(&ini).unwrap();
        assert!(written.starts_with(INI_MARKER));
        // ONE key here is true whatever is installed, and this test used to
        // claim two. The icon set is not asserted: the bundled theme names
        // `default`, which is a cursor redirect rather than an icon theme, so
        // on most machines the key is correctly absent and a test demanding it
        // would be demanding the bug back.
        assert!(written.contains("gtk-cursor-theme-name="));

        // The FONT is host-dependent and the old comment said it was not, which
        // is how the assertion below came to fail on CI while passing here: a
        // GitHub runner has no Inter, the emitter correctly drops a family
        // fontconfig cannot resolve (`a_font_this_machine_cannot_resolve_is_not_named`
        // is the same rule from the other side), and the test read that as a
        // defect. So assert the CONTRACT rather than the machine - the key is
        // there exactly when the family resolves, mirroring the gate in
        // `write_foreign_toolkit_configs` including the "an absent `fc-match`
        // says nothing" arm.
        let fixture = theme();
        let family = crate::wine::first_family(&fixture.typography.font_sans);
        if crate::gtk::font_family_installed(family) == Some(false) {
            assert!(
                !written.contains("gtk-font-name="),
                "{family:?} does not resolve here, so it must not be named:\n{written}"
            );
        } else {
            assert!(
                written.contains("gtk-font-name="),
                "{family:?} resolves here, so it must be named:\n{written}"
            );
        }

        // GTK 4 reads its own file, so the same answer has to be in both. The
        // difference is the theme name: GTK3 may be told which widget theme to
        // use, GTK4 never is, because we ship none.
        let ini4 = tmp.path().join("gtk-4.0/settings.ini");
        assert!(report.written.contains(&ini4), "{report:?}");
        let four = std::fs::read_to_string(&ini4).unwrap();
        assert!(four.starts_with(INI_MARKER));
        assert!(four.contains("gtk-cursor-theme-size="));
        assert!(four.contains("gtk-application-prefer-dark-theme="));
        assert!(!four.contains("gtk-theme-name"), "GTK4 must not be told a theme: {four}");
        // Written a second time over its own file rather than skipped.
        let again = write_foreign_toolkit_configs(&theme(), tmp.path());
        assert!(again.written.contains(&ini));
        assert!(!again.skipped_foreign.contains(&ini));
    }

    /// The scheme is written and something points at it. Both, or the palette is
    /// a file nobody reads.
    #[test]
    fn the_qt_scheme_is_selected_and_not_just_written() {
        let tmp = tempfile::TempDir::new().unwrap();
        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        for dir in ["qt6ct", "qt5ct"] {
            let scheme = tmp.path().join(dir).join("colors/arlen.conf");
            let select = tmp.path().join(dir).join(format!("{dir}.conf"));
            assert!(report.written.contains(&scheme), "{dir} scheme: {report:?}");
            assert!(report.written.contains(&select), "{dir} selection: {report:?}");
            let text = std::fs::read_to_string(&select).unwrap();
            assert!(text.starts_with(INI_MARKER));
            assert!(text.contains("custom_palette=true"));
            assert!(
                text.contains(&format!("color_scheme_path={}", scheme.display())),
                "the selection must name the scheme beside it: {text}"
            );
        }
    }

    /// And a qt6ct.conf the person already has is theirs. qt6ct rewrites this
    /// file whenever somebody uses its window, so a foreign one is the normal
    /// case on a machine that has ever run it.
    #[test]
    fn a_users_own_qt6ct_conf_is_not_clobbered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conf = tmp.path().join("qt6ct/qt6ct.conf");
        std::fs::create_dir_all(conf.parent().unwrap()).unwrap();
        let mine = "[Appearance]\nstyle=Breeze\n";
        std::fs::write(&conf, mine).unwrap();

        let report = write_foreign_toolkit_configs(&theme(), tmp.path());
        assert!(report.skipped_foreign.contains(&conf));
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), mine);
        // The scheme is still written: it is our own file under `colors/`.
        assert!(report.written.contains(&tmp.path().join("qt6ct/colors/arlen.conf")));
    }

    /// The three states, over the files a person can really have.
    #[test]
    fn the_reach_says_whether_the_theme_is_actually_in_place() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Nothing applied yet.
        let before = toolkit_reach(tmp.path());
        assert_eq!(before["gtk3"], ToolkitReach::Absent);
        assert_eq!(before["terminal"], ToolkitReach::Absent);

        // After an apply, ours.
        write_foreign_toolkit_configs(&theme(), tmp.path());
        let after = toolkit_reach(tmp.path());
        for toolkit in ["gtk3", "gtk4", "qt", "terminal"] {
            assert_eq!(after[toolkit], ToolkitReach::Ours, "{toolkit}");
        }

        // One file of somebody's own blocks that toolkit and only that one.
        std::fs::write(tmp.path().join("gtk-3.0/settings.ini"), "[Settings]\n").unwrap();
        let blocked = toolkit_reach(tmp.path());
        assert_eq!(
            blocked["gtk3"],
            ToolkitReach::Blocked(tmp.path().join("gtk-3.0/settings.ini")),
            "and it names the file in the way"
        );
        assert_eq!(blocked["gtk4"], ToolkitReach::Ours, "one toolkit at a time");
        assert_eq!(blocked["terminal"], ToolkitReach::Ours);
    }

    /// Half is not reach. A toolkit whose colours are ours and whose settings
    /// file is not is following half a theme, and the badge that says "full"
    /// over that is the thing this exists to stop.
    #[test]
    fn one_foreign_file_blocks_the_toolkit_even_beside_our_own() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("gtk-3.0")).unwrap();
        std::fs::write(
            tmp.path().join("gtk-3.0/gtk.css"),
            format!("{GTK_HEADER}@define-color window_bg_color #000;\n"),
        )
        .unwrap();
        std::fs::write(tmp.path().join("gtk-3.0/settings.ini"), "[Settings]\n").unwrap();
        assert_eq!(
            toolkit_reach(tmp.path())["gtk3"],
            ToolkitReach::Blocked(tmp.path().join("gtk-3.0/settings.ini"))
        );
    }

    /// A switched-off spoke gets no files, and the ones beside it still do. The
    /// pair matters: a switch that quietly took the whole apply with it would
    /// pass a test that only looked at the spoke it turned off.
    #[test]
    fn a_switched_off_spoke_is_not_written_and_its_neighbours_are() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let t = theme();
        write_toolkit_configs(&t, &t, &t, &t, &[Spoke::Qt], tmp.path());

        assert!(
            !tmp.path().join("qt6ct/qt6ct.conf").exists(),
            "Qt was switched off and its selection file was written anyway"
        );
        assert!(
            !tmp.path().join("qt6ct/colors/arlen.conf").exists(),
            "Qt was switched off and its palette was written anyway"
        );
        assert!(
            tmp.path().join("gtk-3.0/gtk.css").is_file(),
            "switching Qt off took GTK3 with it"
        );
        assert!(
            tmp.path().join("kitty/arlen-colors.conf").is_file(),
            "switching Qt off took the terminals with it"
        );
    }

    /// Switching a spoke off after it was written takes back the files that
    /// STEER it, and leaves the palette beside them. The palette is the half a
    /// hand-written include points at, and removing it would break a config
    /// somebody else wrote - see `steering_files`.
    #[test]
    fn switching_off_takes_back_the_steering_files_and_leaves_the_palette() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let t = theme();
        write_toolkit_configs(&t, &t, &t, &t, &[], tmp.path());
        assert!(tmp.path().join("kitty/kitty.conf").is_file(), "no include to take back");

        write_toolkit_configs(&t, &t, &t, &t, &[Spoke::Terminal], tmp.path());
        assert!(
            !tmp.path().join("kitty/kitty.conf").exists(),
            "the include we wrote still points kitty at the theme"
        );
        assert!(
            tmp.path().join("kitty/arlen-colors.conf").is_file(),
            "the palette was removed; an include somebody wrote themselves now names a missing file"
        );
    }

    /// And it takes back only OUR files. Somebody else's `kitty.conf` is theirs,
    /// switch or no switch: the apply already refuses to overwrite it, and
    /// deleting what it would not overwrite would be the worse half of the same
    /// mistake.
    #[test]
    fn switching_off_never_removes_a_file_somebody_else_wrote() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let theirs = tmp.path().join("kitty/kitty.conf");
        std::fs::create_dir_all(theirs.parent().unwrap()).expect("mkdir");
        std::fs::write(&theirs, "font_size 12\n").expect("write");

        let t = theme();
        write_toolkit_configs(&t, &t, &t, &t, &[Spoke::Terminal], tmp.path());
        assert_eq!(
            std::fs::read_to_string(&theirs).expect("read"),
            "font_size 12\n"
        );
    }

    /// The row id is the name on both sides of the bridge, so the mapping has to
    /// round-trip - and a row that is not a spoke has to answer `None` rather
    /// than land on one.
    #[test]
    fn a_row_id_names_at_most_one_spoke() {
        for spoke in [Spoke::Gtk3, Spoke::Gtk4, Spoke::Qt, Spoke::Terminal] {
            assert_eq!(Spoke::from_id(spoke.id()), Some(spoke));
        }
        assert_eq!(Spoke::from_id("arlen"), None);
        assert_eq!(Spoke::from_id("wine"), None);
    }

}
