//! The boot-verify hook: which app, if any, this boot was asked to launch.
//!
//! The harness passes an app's binary name in the SMBIOS system SKU
//! (`-smbios type=1,sku=<binary>`), which the kernel's DMI driver exposes
//! world-readable at `/sys/class/dmi/id/product_sku`. QEMU leaves it empty by
//! default, so a normal boot launches nothing and real hardware is unaffected.
//!
//! The value comes from outside, so it is sanitised to the app-name charset and
//! must resolve to an installed binary: it can only ever start a real app, never
//! inject a command. That is the whole security content of this module, which is
//! why it is a pure function with the charset written down rather than a `tr` in a
//! pipeline.

/// The characters an app binary name may contain. Anything else is dropped, so a
/// SKU carrying a space, a semicolon or a slash cannot become an argument, a
/// second command or a path.
fn is_app_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// The app name a SKU asks for, or `None` when it asks for nothing.
///
/// Sanitise-then-check, never check-then-sanitise: the value that is looked up
/// has to be the value that runs, or the check is about a different string.
pub fn requested_app(sku: &str) -> Option<String> {
    let name: String = sku.chars().filter(|c| is_app_name_char(*c)).collect();
    (!name.is_empty()).then_some(name)
}

/// The FILE a verify boot asks the app to open, or `None`.
///
/// Rides in the SMBIOS product VERSION beside the app name in the SKU, because
/// the SKU's charset deliberately cannot carry a path - that charset is the
/// reason a SKU can never become an argument, and widening it would spend the
/// property this module exists for.
///
/// The rule here is narrower than "a string": an ABSOLUTE path, one line, no
/// `..` segment, and nothing but the characters a path needs. It is handed to
/// the app as a single argv element and never through a shell, so a value that
/// gets this far can name a file and nothing else - no second command, no
/// redirection, no word splitting. The caller still checks the file EXISTS,
/// because a launch that opens nothing is a boot that quietly proves nothing.
pub fn requested_file(version: &str) -> Option<String> {
    let line = version.trim();
    if !line.starts_with('/') || line.len() > 4096 {
        return None;
    }
    if line.chars().any(|c| c.is_control() || c == '\0') {
        return None;
    }
    if line.split('/').any(|part| part == "..") {
        return None;
    }
    Some(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_sku_asks_for_nothing() {
        // Every normal boot and all real hardware.
        assert_eq!(requested_app(""), None);
        assert_eq!(requested_app("\n"), None);
        assert_eq!(requested_app("!!!"), None, "nothing survives the charset");
    }

    #[test]
    fn an_ordinary_app_name_survives_intact() {
        assert_eq!(requested_app("arlen-files"), Some("arlen-files".to_string()));
        assert_eq!(requested_app("arlen-files\n"), Some("arlen-files".to_string()));
    }

    #[test]
    fn nothing_in_a_sku_can_become_a_second_command_or_a_path() {
        // The injection shapes, each reduced to an inert name that then has to
        // resolve to an installed binary before anything runs.
        for hostile in [
            "arlen-files; rm -rf /",
            "arlen-files && curl evil",
            "../../usr/bin/sh",
            "arlen files",
            "$(id)",
        ] {
            let got = requested_app(hostile).unwrap_or_default();
            for bad in [';', '&', '/', ' ', '$', '(', ')', '.'] {
                assert!(!got.contains(bad), "{hostile:?} kept {bad:?}: {got:?}");
            }
        }
    }

    #[test]
    fn a_file_must_be_one_absolute_path_and_nothing_else() {
        assert_eq!(
            requested_file("/home/arlen/rechnung.eml"),
            Some("/home/arlen/rechnung.eml".into())
        );
        // The whole point of the separate field: what the SKU may not carry.
        assert_eq!(requested_file("rechnung.eml"), None, "relative");
        assert_eq!(requested_file(""), None);
        assert_eq!(requested_file("   "), None);
        assert_eq!(requested_file("/home/../etc/shadow"), None, "traversal");
        assert_eq!(requested_file("/tmp/a\nrm -rf /"), None, "a second line");
        assert_eq!(requested_file("/tmp/a\0b"), None, "a NUL");
    }

    #[test]
    fn a_path_with_spaces_is_still_one_path() {
        // It is handed over as one argv element, so a space is a character in a
        // filename rather than a word break - the reason this is not sanitised
        // to the app-name charset.
        assert_eq!(
            requested_file("/home/arlen/Meine Datei.eml"),
            Some("/home/arlen/Meine Datei.eml".into())
        );
    }
}
