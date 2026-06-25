//! Open-with support: parsing freedesktop `.desktop` entries and matching them
//! to a file's MIME type, for the FM "Open With" picker. Pure and
//! dependency-free - it reads only the handful of keys the picker needs, so the
//! core takes no desktop-entry crate. The host (src-tauri) does the I/O: scan
//! the application directories, query the file's MIME type, feed each entry's
//! text here, and expand the `Exec` field codes at launch. This module is the
//! testable decision layer: what is a launchable app, and does it handle a type.

/// A launchable application parsed from a `.desktop` entry's `[Desktop Entry]`
/// group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopApp {
    /// The desktop-file id (its basename, e.g. `org.gnome.gedit.desktop`). It
    /// identifies the entry in `mimeapps.list`, so the default handler can be
    /// matched and surfaced first in the picker.
    pub id: String,
    /// `Name=` - the display label.
    pub name: String,
    /// `Exec=` verbatim, with its field codes (`%f`, `%u`, ...) intact; the host
    /// expands them against the target path at launch.
    pub exec: String,
    /// `MimeType=` values (`;`-separated in the file), lower-cased, empties
    /// dropped.
    pub mime_types: Vec<String>,
    /// `Terminal=true` - the launcher must run it inside a terminal.
    pub terminal: bool,
}

/// Parse the `[Desktop Entry]` group of a `.desktop` file into a launchable app,
/// or `None` when it is not one: a `Type` other than `Application`, `NoDisplay`
/// or `Hidden` set, or a missing/empty `Name` or `Exec`. Only the first group is
/// read; later `[Desktop Action ...]` groups are ignored. Locale-suffixed keys
/// (`Name[de]`) are skipped, so the unlocalised value wins.
pub fn parse_desktop_app(id: &str, contents: &str) -> Option<DesktopApp> {
    let mut in_entry = false;
    let mut seen_entry = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut mime: Vec<String> = Vec::new();
    let mut no_display = false;
    let mut hidden = false;
    let mut typ: Option<String> = None;
    let mut terminal = false;

    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            // A group header. Once the Desktop Entry group has been read, any
            // following group (e.g. a Desktop Action) ends our scan.
            if seen_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            seen_entry = in_entry;
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Name" => {
                if name.is_none() {
                    name = Some(value.to_string());
                }
            }
            "Exec" => exec = Some(value.to_string()),
            "MimeType" => {
                mime = value
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_lowercase)
                    .collect();
            }
            "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "Type" => typ = Some(value.to_string()),
            "Terminal" => terminal = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    if no_display || hidden {
        return None;
    }
    if matches!(&typ, Some(t) if t != "Application") {
        return None;
    }
    let name = name.filter(|s| !s.is_empty())?;
    let exec = exec.filter(|s| !s.is_empty())?;
    Some(DesktopApp {
        id: id.to_string(),
        name,
        exec,
        mime_types: mime,
        terminal,
    })
}

/// The user's default handler for `mime`, as a desktop-file id, from a
/// `mimeapps.list` text. Reads the `[Default Applications]` group, where each
/// line is `mime/type=app.desktop` (a `;`-separated list, first entry wins).
/// `None` when the type has no default set. Pure: the host reads the file.
pub fn default_app_for(mimeapps_text: &str, mime: &str) -> Option<String> {
    let mime = mime.to_lowercase();
    let mut in_defaults = false;
    for raw in mimeapps_text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_defaults = line == "[Default Applications]";
            continue;
        }
        if !in_defaults {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().to_lowercase() == mime {
                return value
                    .split(';')
                    .map(str::trim)
                    .find(|s| !s.is_empty())
                    .map(str::to_string);
            }
        }
    }
    None
}

/// Whether `app` declares (exactly) that it handles `mime`. Subclass/alias
/// resolution (a `text/x-rust` file also being `text/plain`) is the host's job
/// via the MIME system; this is the declared-match the picker filters on.
pub fn app_handles_mime(app: &DesktopApp, mime: &str) -> bool {
    let mime = mime.to_lowercase();
    app.mime_types.iter().any(|m| *m == mime)
}

/// The apps from `apps` that handle `mime`, de-duplicated by `Exec` (the same
/// app installed twice). Sorted by `Name` (case-insensitive), except the user's
/// default handler (`default_id`, a desktop-file id) is pulled to the front so
/// the picker leads with it - the freedesktop expectation. `default_id` of
/// `None`, or one that does not handle the type, leaves the plain alphabetical
/// order.
pub fn apps_for_mime(apps: &[DesktopApp], mime: &str, default_id: Option<&str>) -> Vec<DesktopApp> {
    let mut out: Vec<DesktopApp> = Vec::new();
    for app in apps.iter().filter(|a| app_handles_mime(a, mime)) {
        if !out.iter().any(|k| k.exec == app.exec) {
            out.push(app.clone());
        }
    }
    out.sort_by(|a, b| {
        let a_default = Some(a.id.as_str()) == default_id;
        let b_default = Some(b.id.as_str()) == default_id;
        // The default leads; otherwise case-insensitive by name.
        b_default
            .cmp(&a_default)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Expand a `.desktop` `Exec=` string into an argv for launching `file`, per the
/// freedesktop field-code rules. The single-file/url codes (`%f` `%u` `%F` `%U`)
/// become the file path as one argument; `%%` is a literal `%`; the icon/name/
/// deprecated codes (`%i` `%c` `%k` `%d` `%D` `%n` `%N` `%v` `%m`) are dropped.
/// The result is an argv the host spawns directly (no shell), so a path with
/// spaces or shell metacharacters is one inert argument - never re-parsed by a
/// shell. Apps in the picker come from MimeType-declaring entries, which carry a
/// file code; an Exec with no code simply launches without the file (the spec's
/// behaviour - we do not append it).
pub fn expand_exec(exec: &str, file: &str) -> Vec<String> {
    tokenize_exec(exec)
        .into_iter()
        .filter_map(|tok| match tok.as_str() {
            "%f" | "%u" | "%F" | "%U" => Some(file.to_string()),
            "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => None,
            _ => Some(expand_codes_inline(&tok, file)),
        })
        .collect()
}

/// Replace field codes embedded inside a token (e.g. `--file=%f`): the single-
/// file codes become `file`, `%%` a literal `%`, any other `%x` is dropped.
fn expand_codes_inline(tok: &str, file: &str) -> String {
    let mut out = String::new();
    let mut chars = tok.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => out.push('%'),
                Some('f') | Some('u') | Some('F') | Some('U') => out.push_str(file),
                _ => {} // drop other / truncated codes
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split an `Exec=` string into tokens, honouring double-quoted segments (a
/// quoted run is one argument; inside it `\"` `\\` `\$` `` \` `` unescape per the
/// spec). Whitespace separates unquoted tokens.
fn tokenize_exec(exec: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut active = false;
    let mut in_quote = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quote {
            match c {
                '\\' => {
                    if let Some(&n) = chars.peek() {
                        if matches!(n, '"' | '\\' | '$' | '`') {
                            cur.push(n);
                            chars.next();
                            continue;
                        }
                    }
                    cur.push('\\');
                }
                '"' => in_quote = false,
                _ => cur.push(c),
            }
        } else if c == '"' {
            in_quote = true;
            active = true;
        } else if c.is_whitespace() {
            if active {
                tokens.push(std::mem::take(&mut cur));
                active = false;
            }
        } else {
            cur.push(c);
            active = true;
        }
    }
    if active {
        tokens.push(cur);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "[Desktop Entry]\n\
        Type=Application\n\
        Name=Firefox\n\
        Name[de]=Feuerfuchs\n\
        Exec=firefox %u\n\
        Terminal=false\n\
        MimeType=text/html;application/xhtml+xml;x-scheme-handler/http;\n\
        [Desktop Action new-window]\n\
        Name=New Window\n\
        Exec=firefox --new-window\n";

    #[test]
    fn parses_the_entry_group_only() {
        let app = parse_desktop_app("firefox.desktop", FIREFOX).unwrap();
        assert_eq!(app.name, "Firefox"); // not the localised Name[de], not the action's Name
        assert_eq!(app.exec, "firefox %u");
        assert!(!app.terminal);
        assert!(app.mime_types.contains(&"text/html".to_string()));
        assert!(app.mime_types.contains(&"x-scheme-handler/http".to_string()));
    }

    #[test]
    fn skips_non_application_and_hidden_entries() {
        assert!(parse_desktop_app("l.desktop", "[Desktop Entry]\nType=Link\nName=L\nExec=x\n").is_none());
        assert!(
            parse_desktop_app("n.desktop", "[Desktop Entry]\nName=N\nExec=x\nNoDisplay=true\n").is_none()
        );
        assert!(parse_desktop_app("n.desktop", "[Desktop Entry]\nName=N\nExec=x\nHidden=true\n").is_none());
    }

    #[test]
    fn requires_name_and_exec() {
        assert!(parse_desktop_app("x.desktop", "[Desktop Entry]\nType=Application\nExec=x\n").is_none());
        assert!(parse_desktop_app("n.desktop", "[Desktop Entry]\nType=Application\nName=N\n").is_none());
    }

    #[test]
    fn terminal_flag_and_no_mimetype() {
        let app = parse_desktop_app("vim.desktop", "[Desktop Entry]\nName=Vim\nExec=vim %F\nTerminal=true\n")
            .unwrap();
        assert!(app.terminal);
        assert!(app.mime_types.is_empty());
    }

    #[test]
    fn expand_exec_substitutes_the_single_file_codes() {
        assert_eq!(expand_exec("firefox %u", "/a/b.html"), ["firefox", "/a/b.html"]);
        assert_eq!(expand_exec("vim %F", "/x.txt"), ["vim", "/x.txt"]);
        assert_eq!(
            expand_exec("app --flag %f", "/p"),
            ["app", "--flag", "/p"]
        );
    }

    #[test]
    fn expand_exec_drops_icon_and_deprecated_codes_and_keeps_literal_percent() {
        // %i/%c/%k dropped (not empty args); %% -> literal %.
        assert_eq!(expand_exec("app %i %c %f", "/p"), ["app", "/p"]);
        assert_eq!(expand_exec("app 100%% %f", "/p"), ["app", "100%", "/p"]);
    }

    #[test]
    fn expand_exec_keeps_a_path_with_spaces_as_one_inert_arg() {
        // The argv is spawned without a shell, so a space/metachar in the path is
        // one argument, never re-split or interpreted.
        let argv = expand_exec("viewer %f", "/home/me/My Photos/a b;rm -rf.png");
        assert_eq!(argv, ["viewer", "/home/me/My Photos/a b;rm -rf.png"]);
    }

    #[test]
    fn expand_exec_handles_a_quoted_program_with_spaces() {
        assert_eq!(
            expand_exec("\"/opt/My App/run\" %f", "/p"),
            ["/opt/My App/run", "/p"]
        );
    }

    #[test]
    fn expand_exec_substitutes_an_inline_code() {
        assert_eq!(expand_exec("app --file=%f", "/p"), ["app", "--file=/p"]);
    }

    #[test]
    fn matches_mime_case_insensitively() {
        let app = parse_desktop_app("firefox.desktop", FIREFOX).unwrap();
        assert!(app_handles_mime(&app, "TEXT/HTML"));
        assert!(!app_handles_mime(&app, "image/png"));
    }

    #[test]
    fn filters_dedups_and_sorts() {
        let html = parse_desktop_app(
            "zed.desktop",
            "[Desktop Entry]\nName=Zed Browser\nExec=zed %u\nMimeType=text/html;\n",
        )
        .unwrap();
        let html2 = parse_desktop_app(
            "zed.desktop",
            "[Desktop Entry]\nName=Zed Browser\nExec=zed %u\nMimeType=text/html;\n",
        )
        .unwrap();
        let ff = parse_desktop_app("firefox.desktop", FIREFOX).unwrap();
        let img = parse_desktop_app(
            "eog.desktop",
            "[Desktop Entry]\nName=Eye\nExec=eog %f\nMimeType=image/png;\n",
        )
        .unwrap();
        let got = apps_for_mime(&[html, ff.clone(), html2, img], "text/html", None);
        // image app excluded; the duplicate Exec collapsed; sorted by name.
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "Firefox");
        assert_eq!(got[1].name, "Zed Browser");
    }

    #[test]
    fn the_default_handler_leads_the_picker() {
        let zed = parse_desktop_app(
            "zed.desktop",
            "[Desktop Entry]\nName=Zed Browser\nExec=zed %u\nMimeType=text/html;\n",
        )
        .unwrap();
        let ff = parse_desktop_app("firefox.desktop", FIREFOX).unwrap();
        // Firefox sorts after Zed alphabetically, but the user's default wins the lead.
        let got = apps_for_mime(&[zed, ff], "text/html", Some("firefox.desktop"));
        assert_eq!(got[0].id, "firefox.desktop");
        assert_eq!(got[1].name, "Zed Browser");
        // A default that does not handle the type leaves the plain order.
        let zed2 = parse_desktop_app(
            "zed.desktop",
            "[Desktop Entry]\nName=Zed Browser\nExec=zed %u\nMimeType=text/html;\n",
        )
        .unwrap();
        let ff2 = parse_desktop_app("firefox.desktop", FIREFOX).unwrap();
        let plain = apps_for_mime(&[zed2, ff2], "text/html", Some("nano.desktop"));
        assert_eq!(plain[0].name, "Firefox");
    }

    #[test]
    fn default_app_for_reads_the_default_applications_group() {
        let mimeapps = "[Default Applications]\n\
            text/html=firefox.desktop;chromium.desktop\n\
            image/png=org.gnome.eog.desktop\n\
            [Added Associations]\n\
            text/html=otherbrowser.desktop\n";
        // First entry of a `;`-list wins; case-insensitive on the mime key.
        assert_eq!(
            default_app_for(mimeapps, "TEXT/HTML").as_deref(),
            Some("firefox.desktop")
        );
        assert_eq!(
            default_app_for(mimeapps, "image/png").as_deref(),
            Some("org.gnome.eog.desktop")
        );
        // A type with no default, and the Added Associations group, are ignored.
        assert!(default_app_for(mimeapps, "application/pdf").is_none());
    }
}
