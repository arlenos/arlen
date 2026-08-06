//! Which language the user chose.
//!
//! One reader, because there are several: the shell plugin serves it to every
//! app, the desktop shell reads it for itself (it embeds no plugin, being the
//! shell), and anything else that wants to render in the user's language needs
//! the same answer. Two copies of a four-line predicate would be two answers the
//! first time one of them was edited.

use std::path::PathBuf;

/// The language the messages are authored in, and the floor of every fallback.
pub const SOURCE_LOCALE: &str = "en";

/// `~/.config/arlen/locale.toml`, where Settings writes the choice.
pub fn locale_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen")
        .join("locale.toml")
}

/// The chosen UI language tag, or [`SOURCE_LOCALE`] when nothing has chosen.
///
/// Deliberately not `LANG`. A machine nobody has told otherwise gets the language
/// the messages were written in, rather than a guess from a locale that may have
/// been set for number and date formats alone - switching somebody's whole
/// desktop on that inference is a worse default than leaving it in English.
pub fn chosen_locale() -> String {
    read_chosen(&locale_config_path())
}

/// The reader over an explicit path, so it can be tested against a real file.
pub fn read_chosen(path: &std::path::Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return SOURCE_LOCALE.to_string();
    };
    let Ok(doc) = text.parse::<toml::Table>() else {
        return SOURCE_LOCALE.to_string();
    };
    doc.get("locale")
        .and_then(|l| l.get("ui"))
        .and_then(|v| v.as_str())
        .filter(|tag| is_locale_tag(tag))
        .unwrap_or(SOURCE_LOCALE)
        .to_string()
}

/// A BCP-47-shaped tag, loosely.
///
/// The value reaches a catalog lookup and an `Intl` constructor, so anything else
/// is refused here rather than passed on to find out what happens.
pub fn is_locale_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 35
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_letters_digits_and_hyphens() {
        assert!(is_locale_tag("de"));
        assert!(is_locale_tag("zh-Hant-TW"));
        assert!(!is_locale_tag(""));
        assert!(!is_locale_tag("../etc"));
        // POSIX, not BCP-47: the encoding suffix and the underscore both.
        assert!(!is_locale_tag("de_AT.UTF-8"));
        assert!(!is_locale_tag(&"x".repeat(36)));
    }

    #[test]
    fn a_choice_is_read_and_anything_else_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("locale.toml");

        // Nothing written yet.
        assert_eq!(read_chosen(&p), SOURCE_LOCALE);

        std::fs::write(&p, "[locale]\nui = \"de-AT\"\n").unwrap();
        assert_eq!(read_chosen(&p), "de-AT");

        // Present but not a tag: fall back rather than hand it on.
        std::fs::write(&p, "[locale]\nui = \"../../etc/passwd\"\n").unwrap();
        assert_eq!(read_chosen(&p), SOURCE_LOCALE);

        // Present but not TOML.
        std::fs::write(&p, "locale = [\n").unwrap();
        assert_eq!(read_chosen(&p), SOURCE_LOCALE);
    }
}
