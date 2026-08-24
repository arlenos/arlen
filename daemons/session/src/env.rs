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

    // The log level, when somebody set one on the session.
    //
    // A release compositor logs at `warn` and there was no way to raise it: the
    // session builds this map and spawns the compositor with exactly it, so a
    // `RUST_LOG` set anywhere outside reached the session process and stopped
    // there. That made a whole class of question unanswerable on a real image -
    // when a window first reached the screen, why an input was refused - because
    // the only process that knows is the one whose voice was turned down.
    //
    // Inherited rather than defaulted: an image that says nothing keeps the quiet
    // release level, and raising it stays a deliberate act (a unit drop-in, a
    // debug boot), not something a build decides for every user.
    if let Ok(v) = std::env::var("RUST_LOG") {
        if !v.is_empty() {
            env.insert("RUST_LOG".into(), v);
        }
    }
    env
}

/// Where the image records the POSIX locales it actually generated, one per
/// line, most-preferred first.
///
/// WRITTEN BY THE BUILD, not by hand: the build lists the finished locale archive
/// and writes what is really in it. A hand-kept list would be a claim, and the
/// failure it causes is silent - glibc handed a locale that was never generated
/// falls back to C and formats American, which is exactly the bug this whole path
/// exists to fix, now with a variable set that says otherwise.
pub const GENERATED_LOCALES: &str = "/usr/share/arlen/locales";

/// The locales the machine generated, or nothing at all.
///
/// A missing file is the normal answer on any system that is not our image, and
/// it means "export no language" - the behaviour every boot had before this.
pub fn read_generated_locales(path: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| {
            line.len() <= 32
                && line
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        })
        .map(str::to_string)
        .collect()
}

/// The POSIX locale to run the session under, given the UI language the user
/// chose and the locales this machine has.
///
/// WHY THE SESSION SETS A LOCALE AT ALL. The webview renders our own text from the
/// message catalogues and formats its own dates through `Intl` with the tag passed
/// explicitly, so all of that is right whatever the process locale is. But the
/// NATIVE controls are not ours: WebKitGTK renders `<input type="date">` and
/// `<input type="time">` through the C library's locale, and with none set that is
/// C - American. Measured in the same renderer on 24 Aug: under `C` the calendar's
/// date field reads `08/24/2026` and its times `09:00 AM`, under `de_AT.UTF-8` the
/// same page reads `24.08.2026` and `09:00`. A German desktop was writing American
/// dates in every form, and no amount of catalogue work reaches them.
///
/// ONLY WHAT THE MACHINE HAS. glibc given a locale it never generated does not
/// complain in the session log anybody reads; it silently falls back to C. So a
/// name is exported only when it is in the machine's own list, and a language with
/// nothing generated for it keeps the old behaviour rather than a variable that
/// lies.
///
/// THE REGION COMES FROM THE IMAGE. `de` is a language, `de_DE.UTF-8` is a locale,
/// and nothing in a bare tag says which country's conventions to use. Rather than
/// keep a tag-to-region table here that would need editing every time a language
/// is added, the first generated locale for that language wins - so the image's
/// list, in its order, is what decides, next to the line that generates it.
pub fn posix_locale_for(tag: &str, generated: &[String]) -> Option<String> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    let mut parts = tag.split('-');
    let language = parts.next()?.to_ascii_lowercase();
    if language.is_empty() || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let region = parts
        .next()
        .filter(|r| r.len() == 2 && r.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_ascii_uppercase);

    // The language of a POSIX name: everything before `_`, `.` or `@`.
    let language_of = |name: &str| {
        name.split(['_', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let region_of = |name: &str| {
        name.split('_')
            .nth(1)
            .map(|rest| rest.split(['.', '@']).next().unwrap_or(""))
            .map(str::to_ascii_uppercase)
    };

    // The chosen region first, when the tag named one and the machine has it.
    if let Some(region) = &region {
        if let Some(exact) = generated.iter().find(|name| {
            language_of(name) == language && region_of(name).as_deref() == Some(region)
        }) {
            return Some(exact.clone());
        }
    }
    // Otherwise the image's first locale for that language.
    generated
        .iter()
        .find(|name| language_of(name) == language)
        .cloned()
}

/// The language variables, when the machine has a locale for the chosen language.
///
/// THREE, and the split is the whole point. `LANG` is the category default, so it
/// settles the dates and times the native controls draw. `LC_MESSAGES` and
/// `LC_NUMERIC` are then pinned back to `C`, because this map reaches every
/// process the session starts and several of them read another program's output:
/// the shell parses `rfkill list` for `Soft blocked: yes`, the audio panel parses
/// `pactl`, the store parses `flatpak`. Those strings are translated the moment a
/// locale exists - measured on the host, `ls` of a missing file answers `cannot
/// access` under `C` and `Zugriff ... nicht möglich` under `de_AT.UTF-8` - and a
/// parser reading the German is a panel that says nothing is there.
///
/// `LC_NUMERIC` is in that list for the same reason one step further down: with it
/// following the language, `printf "%.1f" 1.5` prints `1,5`, and a volume read out
/// of `wpctl` stops parsing as a float.
///
/// WHAT IT COSTS. A third-party GTK program shows its own English messages to a
/// German user, and its numbers with a dot. The right end state is each parser
/// pinning `LC_ALL=C` on the command it spawns - a program that reads another
/// program's output owns that - and then messages can follow the language too.
/// Until they do, the desktop's own panels working is worth more than a translated
/// GIMP menu.
///
/// They land in the map rather than being exported here because the map is what
/// [`import_list`] is derived from, and the apps are systemd user services: a
/// variable that is not in the map reaches the session process and stops.
///
/// One knock-on worth naming: `xdg-user-dirs-update`, which this session already
/// runs, reads `LANG`. A German session will therefore get `~/Dokumente` - which is
/// what that call was put there for, and what its own comment describes wanting.
pub fn language_env(tag: &str, generated: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(name) = posix_locale_for(tag, generated) {
        out.insert("LANG".to_string(), name);
        out.insert("LC_MESSAGES".to_string(), "C".to_string());
        out.insert("LC_NUMERIC".to_string(), "C".to_string());
    }
    out
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
    fn a_language_is_exported_only_when_the_machine_generated_it() {
        let have = vec!["en_US.UTF-8".to_string(), "de_DE.UTF-8".to_string()];

        assert_eq!(
            posix_locale_for("de", &have).as_deref(),
            Some("de_DE.UTF-8"),
            "a bare language takes the image's first locale for it"
        );
        assert_eq!(
            posix_locale_for("en", &have).as_deref(),
            Some("en_US.UTF-8")
        );

        // A region the image does not have falls back to the language rather
        // than exporting a name glibc would drop to C.
        assert_eq!(
            posix_locale_for("de-AT", &have).as_deref(),
            Some("de_DE.UTF-8")
        );

        // ...and is taken exactly when it IS there.
        let with_at = vec!["de_AT.UTF-8".to_string(), "de_DE.UTF-8".to_string()];
        assert_eq!(
            posix_locale_for("de-AT", &with_at).as_deref(),
            Some("de_AT.UTF-8")
        );

        // Nothing generated for the language: say nothing, keep the old boot.
        assert_eq!(posix_locale_for("fr", &have), None);
        assert_eq!(posix_locale_for("de", &[]), None);
        assert_eq!(posix_locale_for("", &have), None);
        assert_eq!(posix_locale_for("../etc", &have), None);

        // The apps are user services, so these have to be map entries to reach
        // them at all - and the two pins have to travel with the language, or the
        // panels that parse `rfkill` and `pactl` start reading German.
        let env = language_env("de", &have);
        assert_eq!(env.get("LANG").map(String::as_str), Some("de_DE.UTF-8"));
        assert_eq!(env.get("LC_MESSAGES").map(String::as_str), Some("C"));
        assert_eq!(env.get("LC_NUMERIC").map(String::as_str), Some("C"));

        // Nothing to set means nothing set, pins included: a session that keeps
        // the C formats does not need to be told to keep them.
        assert!(language_env("fr", &have).is_empty());
    }

    #[test]
    fn the_generated_list_is_read_and_junk_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("locales");

        // Absent is the normal answer off our image, and it means "say nothing".
        assert!(read_generated_locales(&p).is_empty());

        std::fs::write(
            &p,
            "# written by the image build
en_US.UTF-8
de_DE.UTF-8

",
        )
        .unwrap();
        assert_eq!(
            read_generated_locales(&p),
            vec!["en_US.UTF-8".to_string(), "de_DE.UTF-8".to_string()]
        );

        // The value is handed to a child process, so a line that is not a locale
        // name is dropped rather than passed on to find out what happens.
        std::fs::write(
            &p,
            "de_DE.UTF-8
../../etc/passwd
x y
",
        )
        .unwrap();
        assert_eq!(read_generated_locales(&p), vec!["de_DE.UTF-8".to_string()]);
    }

    #[test]
    fn a_log_level_travels_only_when_somebody_set_one() {
        // One test for both directions, for the reason the handoff test gives:
        // `set_var` is process-wide and two tests over one variable race.
        std::env::remove_var("RUST_LOG");
        assert!(
            !session_env("s-1", "").contains_key("RUST_LOG"),
            "an image that says nothing keeps the quiet release level"
        );

        std::env::set_var("RUST_LOG", "cosmic_comp=info");
        let env = session_env("s-1", "");
        std::env::remove_var("RUST_LOG");
        assert_eq!(
            env.get("RUST_LOG").map(String::as_str),
            Some("cosmic_comp=info"),
            "the compositor is spawned with exactly this map, so a level that \
             does not travel here cannot reach it at all"
        );

        // An empty value is not a level. Treating it as one would hand the
        // compositor a filter that parses to nothing and silently drop it back
        // to the default, which reads as "the passthrough is broken".
        std::env::set_var("RUST_LOG", "");
        let env = session_env("s-1", "");
        std::env::remove_var("RUST_LOG");
        assert!(!env.contains_key("RUST_LOG"));
    }

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
            assert_eq!(
                session_env("s", family)["WEBKIT_DISABLE_COMPOSITING_MODE"],
                "1"
            );
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
            assert!(
                imported.contains(name),
                "{name} is exported but never imported"
            );
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
