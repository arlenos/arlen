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
    fn a_name_that_could_write_a_second_directive_is_refused() {
        assert!(!usable_name("office\nDefault evil"));
        assert!(!usable_name("of fice"));
        assert!(!usable_name(""));
        assert!(usable_name("office_2"));
        assert!(usable_name("hp-laserjet.local"));
    }
}
