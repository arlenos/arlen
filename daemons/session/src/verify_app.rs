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
}
