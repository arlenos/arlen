// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The user's own default printer, in the file CUPS reads it from.
//!
//! Settings offered a "make this the default" control that invoked a command no
//! host registered, so the page marked a default the next print would ignore.
//! Making it real has a privilege question in the middle of it, and the answer
//! decides which file this writes:
//!
//!   * The SYSTEM default is `CUPS-Set-Default` over IPP and needs printer-admin
//!     rights. A settings app changing what every account on the machine prints
//!     to, silently, is not a per-user preference.
//!   * The USER default is a line in `lpoptions`, owned by the account, read by
//!     every CUPS client for that user. No privilege, no effect on anyone else.
//!
//! This writes the second, which is what `lpoptions -d` does and what a settings
//! page should mean by "default". A machine-wide default is a separate control
//! with a separate authorisation, not this one wearing a different hat.
//!
//! The format is CUPS's own: whitespace-separated lines beginning `Default` or
//! `Dest`, followed by the destination and any per-destination options. Only the
//! `Default` line is touched; `Dest` lines carry a user's saved options per
//! printer and rewriting them here would silently drop settings this code does
//! not understand.

use std::path::PathBuf;

/// Where CUPS reads a user's options from.
///
/// Modern CUPS prefers `$XDG_CONFIG_HOME/cups/lpoptions` and falls back to
/// `~/.cups/lpoptions`. Writing the XDG path when the legacy one already exists
/// would leave the old file shadowing the new one for older clients, so an
/// existing legacy file wins.
pub fn lpoptions_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let legacy = home.join(".cups/lpoptions");
    if legacy.exists() {
        return Some(legacy);
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(xdg.join("cups/lpoptions"))
}

/// The destination named on the `Default` line, if there is one.
pub fn parse_default(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("Default") {
            // `Default printer/instance` is a valid form; the instance is a
            // named option set, not a different printer.
            return parts.next().map(|d| d.split('/').next().unwrap_or(d).to_string());
        }
    }
    None
}

/// The file's text with `printer` as the default, preserving everything else.
///
/// An existing `Default` line is replaced in place rather than removed and
/// appended, so a file a person has edited keeps its order.
pub fn with_default(text: &str, printer: &str) -> String {
    let line = format!("Default {printer}");
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    for existing in text.lines() {
        if existing.split_whitespace().next() == Some("Default") && !replaced {
            out.push(line.clone());
            replaced = true;
        } else {
            out.push(existing.to_string());
        }
    }
    if !replaced {
        out.push(line);
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// Errors writing the user's default. Hand-written like [`PrintError`], because
/// this crate carries no derive-macro dependency.
#[derive(Debug)]
pub enum UserDefaultError {
    /// No `HOME`, so there is no user configuration directory to write to.
    NoHome,
    /// A printer name that could not be written safely.
    BadName(String),
    /// The file could not be read or written.
    Io(std::io::Error),
}

impl std::fmt::Display for UserDefaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserDefaultError::NoHome => {
                write!(f, "no home directory, so there is no per-user CUPS configuration")
            }
            UserDefaultError::BadName(n) => write!(f, "{n} is not a usable printer name"),
            UserDefaultError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UserDefaultError {}

impl From<std::io::Error> for UserDefaultError {
    fn from(e: std::io::Error) -> Self {
        UserDefaultError::Io(e)
    }
}

/// A CUPS destination name: letters, digits and a few separators, no whitespace.
///
/// Checked rather than trusted because the name arrives from the frontend and
/// lands in a line-oriented file: a name with a newline in it would write a
/// second line that CUPS reads as a directive.
fn usable_name(printer: &str) -> bool {
    !printer.is_empty()
        && printer.len() <= 127
        && printer
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
}

/// Read the user's current default printer name.
pub fn read_default() -> Result<Option<String>, UserDefaultError> {
    let path = lpoptions_path().ok_or(UserDefaultError::NoHome)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(parse_default(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Make `printer` this user's default.
pub fn set_default(printer: &str) -> Result<(), UserDefaultError> {
    if !usable_name(printer) {
        return Err(UserDefaultError::BadName(printer.to_string()));
    }
    let path = lpoptions_path().ok_or(UserDefaultError::NoHome)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    // Write beside and rename, so an interrupted write cannot leave a truncated
    // options file - the same file holds a user's saved per-printer settings.
    let tmp = path.with_extension("arlen-tmp");
    std::fs::write(&tmp, with_default(&existing, printer))?;
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// The options saved for one destination, as `key=value` pairs.
///
/// These are the same per-printer defaults `lpoptions -p <printer> -o k=v`
/// writes: paper size, duplex and colour that this user's jobs get unless a
/// dialog overrides them. The keys are IPP attribute names (`media`, `sides`,
/// `print-color-mode`), because that is what CUPS reads back.
pub fn parse_dest_options(text: &str, printer: &str) -> Vec<(String, String)> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("Dest") {
            continue;
        }
        let Some(dest) = parts.next() else { continue };
        if dest.split('/').next().unwrap_or(dest) != printer {
            continue;
        }
        return parts
            .filter_map(|opt| opt.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
    }
    Vec::new()
}

/// The file's text with `options` saved for `printer`.
///
/// Replaces that destination's `Dest` line and leaves every other line alone,
/// including the `Default` line and other printers' options. Options this code
/// does not set are dropped from THAT line only, which is what `lpoptions -o`
/// does: the line is the set of overrides, not a merge target.
pub fn with_dest_options(text: &str, printer: &str, options: &[(String, String)]) -> String {
    let mut line = format!("Dest {printer}");
    for (k, v) in options {
        line.push(' ');
        line.push_str(k);
        line.push('=');
        line.push_str(v);
    }
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    for existing in text.lines() {
        let mut parts = existing.split_whitespace();
        let is_this_dest = parts.next() == Some("Dest")
            && parts
                .next()
                .map(|d| d.split('/').next().unwrap_or(d) == printer)
                .unwrap_or(false);
        if is_this_dest && !replaced {
            out.push(line.clone());
            replaced = true;
        } else {
            out.push(existing.to_string());
        }
    }
    if !replaced {
        out.push(line);
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// Save this user's options for one printer.
pub fn set_dest_options(
    printer: &str,
    options: &[(String, String)],
) -> Result<(), UserDefaultError> {
    if !usable_name(printer) {
        return Err(UserDefaultError::BadName(printer.to_string()));
    }
    for (k, v) in options {
        // Same reason as the printer name: this is a whitespace-separated line,
        // so a value with a space in it would become another option and a value
        // with a newline another directive.
        if !usable_option(k) || !usable_option(v) {
            return Err(UserDefaultError::BadName(format!("{k}={v}")));
        }
    }
    write_options(printer, options)
}

/// An option key or value: IPP keywords are ASCII words with dashes and dots.
fn usable_option(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 127
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn write_options(printer: &str, options: &[(String, String)]) -> Result<(), UserDefaultError> {
    let path = lpoptions_path().ok_or(UserDefaultError::NoHome)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let tmp = path.with_extension("arlen-tmp");
    std::fs::write(&tmp, with_dest_options(&existing, printer, options))?;
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_line_is_replaced_in_place_and_the_rest_is_kept() {
        let before = "Dest office/duplex media=A4\nDefault kitchen\nDest kitchen media=Letter\n";
        let after = with_default(before, "office");
        assert_eq!(
            after,
            "Dest office/duplex media=A4\nDefault office\nDest kitchen media=Letter\n"
        );
    }

    #[test]
    fn a_file_with_no_default_gains_one_without_losing_its_options() {
        let after = with_default("Dest office media=A4\n", "office");
        assert!(after.contains("Dest office media=A4"), "{after}");
        assert!(after.ends_with("Default office\n"), "{after}");
    }

    #[test]
    fn an_empty_file_becomes_one_line() {
        assert_eq!(with_default("", "office"), "Default office\n");
    }

    #[test]
    fn an_instance_suffix_is_not_part_of_the_printer_name() {
        assert_eq!(parse_default("Default office/duplex\n").as_deref(), Some("office"));
        assert_eq!(parse_default("Dest office\n"), None);
    }

    #[test]
    fn one_printers_options_are_replaced_and_the_others_are_not() {
        let before = "Default office\nDest office media=Letter\nDest kitchen media=A4\n";
        let after = with_dest_options(
            before,
            "office",
            &[("media".into(), "A4".into()), ("sides".into(), "two-sided-long-edge".into())],
        );
        assert_eq!(
            after,
            "Default office\nDest office media=A4 sides=two-sided-long-edge\nDest kitchen media=A4\n"
        );
    }

    #[test]
    fn options_for_a_printer_with_no_line_yet_are_appended() {
        let after = with_dest_options("Default office\n", "office", &[("media".into(), "A4".into())]);
        assert_eq!(after, "Default office\nDest office media=A4\n");
    }

    #[test]
    fn saved_options_read_back() {
        let text = "Dest office media=A4 sides=one-sided\n";
        assert_eq!(
            parse_dest_options(text, "office"),
            vec![("media".into(), "A4".into()), ("sides".into(), "one-sided".into())]
        );
        assert!(parse_dest_options(text, "kitchen").is_empty());
    }

    #[test]
    fn an_option_value_with_whitespace_is_refused() {
        assert!(!usable_option("A4 Letter"));
        assert!(!usable_option("A4\nDefault evil"));
        assert!(usable_option("two-sided-long-edge"));
        assert!(usable_option("print-color-mode"));
    }

    #[test]
    fn a_name_that_could_write_a_second_directive_is_refused() {
        assert!(!usable_name("office\nDefault evil"));
        assert!(!usable_name("of fice"));
        assert!(!usable_name(""));
        assert!(usable_name("office_2"));
        assert!(usable_name("hp-laserjet.local"));
    }
}
