//! The environment a session settles before anything graphical starts.
//!
//! Two groups, and they are unrelated despite sitting together in the script this
//! is ported from.
//!
//! **Where the session's clients find the daemons.** The system daemons are
//! systemd SYSTEM services, so they bind under `/run/arlen`; a user-session client
//! DOES have `XDG_RUNTIME_DIR`, so `os_sdk`'s resolver would send it to
//! `/run/user/<uid>/arlen` and it would find nothing. These overrides are what
//! point it back, and they are read ahead of that default by design.
//!
//! **What the graphics stack may do.** The VM's virtio-gpu has no hardware GL, so
//! Mesa runs on llvmpipe and WebKitGTK has to avoid the GPU paths or the Tauri
//! apps paint black. Harmless on real hardware, and the image is the
//! system-under-test.

use std::collections::BTreeMap;

/// The SMBIOS product family that asks for WebKit's accelerated compositing to be
/// left ON (`-smbios type=1,family=webkit-compositing`).
pub const COMPOSITING_FAMILY: &str = "webkit-compositing";

/// Whether WebKit's accelerated compositing stays enabled for this boot.
///
/// Two flags are involved and they are NOT the same lever, which is the thing to
/// keep straight: `WEBKIT_DISABLE_DMABUF_RENDERER` turns off the GPU/dmabuf path
/// (the one that paints Tauri apps black under software GL) and is always set,
/// while `WEBKIT_DISABLE_COMPOSITING_MODE` turns off accelerated compositing
/// altogether - the path where a removed layer's pixels are left behind. Being
/// able to boot the other way is what says whether a residue is a webview defect
/// or something we asked for, so this is a switch rather than a constant.
///
/// Absent or anything else - every normal boot and all real hardware - compositing
/// is disabled exactly as before.
pub fn compositing_enabled(product_family: &str) -> bool {
    let family: String = product_family
        .chars()
        .filter(|c| c.is_ascii_lowercase() || *c == '-')
        .collect();
    family == COMPOSITING_FAMILY
}

/// The variables a session exports before starting the compositor.
///
/// Returned rather than applied so the set is one testable value: the script this
/// replaces spread them over forty lines of `export`, where a missing one is
/// invisible until something downstream reads the wrong socket.
pub fn session_env(session_id: &str, product_family: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    // NO SOCKET PINS. This exported all three - producer, consumer and knowledge -
    // at /run/arlen, "ahead of the XDG_RUNTIME_DIR default", back when those
    // daemons were system services. Both have moved per-user since, and an export
    // here reaches further than any unit file: it lands in the environment of
    // EVERY process the session starts, so it overrode the SDK's correct
    // resolution for the whole desktop.
    //
    // The knowledge pin had been stale for days before this was found, and the
    // symptom was in plain sight the whole time - the shell logging `list_projects:
    // graph query failed (is knowledge daemon running?)` while the daemon was
    // running perfectly, one path over. It was read as a knowledge problem because
    // that is what the message says.
    //
    // Unset, the SDK derives $XDG_RUNTIME_DIR/arlen/, which is where both daemons
    // now bind. A deployment that really does run them system-wide sets these in
    // the units that need them, where the choice is visible.
    env.insert(crate::session_id::SESSION_ID_VAR.into(), session_id.into());
    env.insert("XDG_CURRENT_DESKTOP".into(), "arlen".into());
    // Software GL, and the WebKit paths that cannot use it.
    env.insert("LIBGL_ALWAYS_SOFTWARE".into(), "1".into());
    env.insert("GALLIUM_DRIVER".into(), "llvmpipe".into());
    env.insert("GDK_BACKEND".into(), "wayland".into());
    env.insert("WEBKIT_DISABLE_DMABUF_RENDERER".into(), "1".into());
    if !compositing_enabled(product_family) {
        env.insert("WEBKIT_DISABLE_COMPOSITING_MODE".into(), "1".into());
    }
    // The greeter's accessibility handoff, if it made one.
    //
    // INHERITED rather than computed: greetd puts it in this process's own
    // environment, and it has to land in the map because the import list is
    // derived from these keys - a variable that is not here reaches the session
    // process and stops, so the shell, a systemd user service, never sees it.
    // That is the whole reason it goes through the map rather than being read
    // straight out of the environment where it is needed.
    //
    // Only when set, and BOTH values travel. Absent means nobody operated the
    // toggle at this login, which the session reads as "keep what your own
    // config says". A `0` is as deliberate as a `1` - somebody reached over and
    // turned it off - so it must not be dropped as if it were the absent case.
    if let Ok(v) = std::env::var(A11Y_SCREEN_READER) {
        if v == "1" || v == "0" {
            env.insert(A11Y_SCREEN_READER.into(), v);
        }
    }
    env
}

/// The greeter's screen-reader handoff: `1`, `0`, or absent. Matches
/// `arlen_greeter_core::A11Y_SCREEN_READER_ENV`; the shell reads it once at
/// session start and writes it to the user's config broker.
pub const A11Y_SCREEN_READER: &str = "ARLEN_A11Y_SCREEN_READER";

/// The compositor's own display, learned only after it publishes its socket - so
/// it is not in [`session_env`] and has to join the import list separately.
pub const WAYLAND_DISPLAY: &str = "WAYLAND_DISPLAY";

/// The variables to hand to the `systemd --user` manager, so the shell and the
/// app user services inherit them.
///
/// DERIVED from what the session exported, plus the display it learned. The script
/// this replaces keeps two hand-written lists - the exports and the
/// `import-environment` arguments - which agree today and have no reason to keep
/// agreeing: adding an export and forgetting the import gives every user service a
/// session missing one variable, and the symptom is a daemon quietly reading the
/// wrong socket rather than an error. One list cannot drift from itself.
pub fn import_list(env: &BTreeMap<String, String>) -> Vec<String> {
    let mut names: Vec<String> = env.keys().cloned().collect();
    names.push(WAYLAND_DISPLAY.to_string());
    names.sort();
    names
}

/// The variables that must be UNSET before the compositor starts.
///
/// cosmic-comp picks its backend from the environment: with `DISPLAY` or
/// `WAYLAND_DISPLAY` set it goes nested (x11/winit, for running inside another
/// session), and otherwise it drives the seat's DRM device directly - which is
/// what a login wants. A set-but-EMPTY `DISPLAY=` is enough to send it down the
/// X11 path, where it fails, so these are unset rather than blanked.
pub const MUST_BE_UNSET: &[&str] = &["DISPLAY", "WAYLAND_DISPLAY"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_greeter_handoff_travels_only_when_it_was_made() {
        // ONE test for both directions on purpose: `set_var` is process-wide,
        // so two tests over the same variable race each other under the default
        // parallel runner - and the one that loses reads the other's value.
        std::env::remove_var(A11Y_SCREEN_READER);
        assert!(
            !session_env("s-1", "").contains_key(A11Y_SCREEN_READER),
            "an untouched login screen must say nothing, not 'off'"
        );

        // The shell is a systemd user service: it sees only what the import
        // list carries. A variable that reaches the session process and stops
        // there is a handoff that silently does nothing.
        std::env::set_var(A11Y_SCREEN_READER, "1");
        let env = session_env("s-1", "");
        assert_eq!(env.get(A11Y_SCREEN_READER).map(String::as_str), Some("1"));
        assert!(import_list(&env).contains(&A11Y_SCREEN_READER.to_string()));

        // A deliberate switch-OFF travels too. Dropping it would make "they
        // turned it off here" indistinguishable from "nobody touched it", and
        // the session would then keep an on-flag they just cleared.
        std::env::set_var(A11Y_SCREEN_READER, "0");
        let env = session_env("s-1", "");
        std::env::remove_var(A11Y_SCREEN_READER);
        assert_eq!(env.get(A11Y_SCREEN_READER).map(String::as_str), Some("0"));
    }

    #[test]
    fn the_session_pins_no_daemon_socket() {
        // INVERTED 15 Aug. This asserted the opposite - that all three point at
        // /run/arlen - with a comment explaining that a user-session client would
        // otherwise "look under /run/user/<uid>/arlen and find nothing". That was
        // true when both daemons were system services and became false when they
        // moved; the test then held the whole desktop to a path nothing binds,
        // which is the strongest possible way for a test to be wrong.
        //
        // Nothing may pin them here, because this environment reaches every
        // process the session starts and so beats any per-unit decision made
        // downstream.
        let env = session_env("s-1", "");
        for var in [
            "ARLEN_KNOWLEDGE_SOCKET",
            "ARLEN_PRODUCER_SOCKET",
            "ARLEN_CONSUMER_SOCKET",
        ] {
            assert!(
                !env.contains_key(var),
                "{var} is pinned here, which overrides the per-user resolution for \
                 every process in the session"
            );
        }
        assert_eq!(env["ARLEN_SESSION_ID"], "s-1");
    }

    #[test]
    fn compositing_is_off_on_every_ordinary_boot() {
        for family in ["", "QEMU", "Standard PC", "\n", "webkit"] {
            assert!(!compositing_enabled(family), "{family:?}");
            assert_eq!(session_env("s", family)["WEBKIT_DISABLE_COMPOSITING_MODE"], "1");
        }
    }

    #[test]
    fn the_family_switch_leaves_compositing_on_and_only_that_flag() {
        assert!(compositing_enabled("webkit-compositing"));
        // Trailing newline from the sysfs read, which is how it actually arrives.
        assert!(compositing_enabled("webkit-compositing\n"));
        let env = session_env("s", "webkit-compositing");
        assert!(!env.contains_key("WEBKIT_DISABLE_COMPOSITING_MODE"));
        // The OTHER flag is not the same lever and stays set: without it the
        // Tauri apps paint black under software GL, switch or no switch.
        assert_eq!(env["WEBKIT_DISABLE_DMABUF_RENDERER"], "1");
    }

    #[test]
    fn everything_exported_is_also_handed_to_the_user_manager() {
        // The drift this replaces: two hand-kept lists, agreeing today for no
        // reason that survives the next variable. A forgotten import is a user
        // service reading the wrong socket, not an error.
        let env = session_env("s", "");
        let imported = import_list(&env);
        for name in env.keys() {
            assert!(imported.contains(name), "{name} is exported but never imported");
        }
        assert!(
            imported.contains(&WAYLAND_DISPLAY.to_string()),
            "the shell cannot connect without the display"
        );
        // And the conditional one follows the condition rather than a second list.
        let on = session_env("s", "webkit-compositing");
        assert!(!import_list(&on).contains(&"WEBKIT_DISABLE_COMPOSITING_MODE".to_string()));
    }

    #[test]
    fn the_display_variables_are_unset_rather_than_blanked() {
        // A set-but-empty DISPLAY sends cosmic-comp down the X11 path, where it
        // fails - so they cannot simply be assigned "".
        assert_eq!(MUST_BE_UNSET, ["DISPLAY", "WAYLAND_DISPLAY"]);
        let env = session_env("s", "");
        for var in MUST_BE_UNSET {
            assert!(!env.contains_key(*var), "{var} must not be exported at all");
        }
    }
}
