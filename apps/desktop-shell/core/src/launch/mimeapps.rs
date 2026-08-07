//! Which application handles a MIME type: the `mimeapps.list` lookup.
//!
//! This is the *resolution* half of launching. It has to live in the same
//! component as the launch itself, because the gap it closes is that today
//! nobody holds both halves: the portal authorises the caller and knows the URI,
//! `xdg-open` knows the handler, and `arlen-run` needs the app id that
//! resolution produces. A launch path where the deciding component and the
//! spawning component are different is a path where the confinement decision can
//! be made without the decider knowing what it decided about.
//!
//! Pure on purpose. The caller supplies the parsed files in precedence order and
//! a predicate for "this desktop entry is installed", so the whole lookup is
//! table-testable without a filesystem, and the parts that must touch the disk
//! (finding the files, checking an entry exists) stay in the host.
//!
//! Spec: <https://specifications.freedesktop.org/mime-apps-spec/latest/>
//!
//! What this does NOT do, so nobody reads more into it: it does not determine a
//! file's MIME type (that is shared-mime-info's job), it does not read the
//! desktop entry's `Exec`, and it does not fall back to `[Added Associations]`
//! when no default is configured. The last one is deliberate: the association
//! list answers "what could open this", which is a picker's question, and a
//! launch that silently picks one is a launch nobody chose.

use std::collections::HashSet;

/// One parsed `mimeapps.list`.
///
/// Only the groups the default lookup reads are kept. `[Added Associations]` is
/// parsed too, because dropping it silently would make a later reader think the
/// file had no such group rather than that we ignored it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MimeApps {
    /// `[Default Applications]`: MIME type to desktop ids, best first.
    pub default: Vec<(String, Vec<String>)>,
    /// `[Added Associations]`: MIME type to desktop ids that can open it.
    pub added: Vec<(String, Vec<String>)>,
    /// `[Removed Associations]`: desktop ids that must not handle the type.
    pub removed: Vec<(String, Vec<String>)>,
}

impl MimeApps {
    fn group(&self, which: Group) -> &[(String, Vec<String>)] {
        match which {
            Group::Default => &self.default,
            Group::Added => &self.added,
            Group::Removed => &self.removed,
        }
    }

    /// The desktop ids listed for `mime` in one group, in file order.
    fn ids(&self, which: Group, mime: &str) -> &[String] {
        self.group(which)
            .iter()
            .find(|(k, _)| k == mime)
            .map_or(&[][..], |(_, v)| v.as_slice())
    }
}

#[derive(Clone, Copy)]
enum Group {
    Default,
    Added,
    Removed,
}

/// Parse one `mimeapps.list`.
///
/// Tolerant by design: an unknown group, a line without `=`, a comment or a
/// blank line is skipped rather than failing the file. A handler map that
/// refuses to load because one line is malformed leaves the user with no
/// handlers at all, which is a worse answer than the rest of their choices.
///
/// A MIME type repeated inside one group keeps the first entry. The spec leaves
/// duplicate keys undefined, and first-wins matches the precedence direction
/// everything else here runs in.
pub fn parse(text: &str) -> MimeApps {
    let mut out = MimeApps::default();
    let mut group: Option<Group> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = match name.trim() {
                "Default Applications" => Some(Group::Default),
                "Added Associations" => Some(Group::Added),
                "Removed Associations" => Some(Group::Removed),
                _ => None,
            };
            continue;
        }
        let Some(g) = group else { continue };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let mime = key.trim().to_string();
        if mime.is_empty() {
            continue;
        }
        let ids: Vec<String> = value
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let target = match g {
            Group::Default => &mut out.default,
            Group::Added => &mut out.added,
            Group::Removed => &mut out.removed,
        };
        if !target.iter().any(|(k, _)| *k == mime) {
            target.push((mime, ids));
        }
    }
    out
}

/// The desktop id that should handle `mime`, or `None` if nothing is configured.
///
/// `files` are the parsed `mimeapps.list` files in precedence order, highest
/// first. `installed` answers whether a desktop id resolves to an entry that
/// actually exists; a default naming an uninstalled application is skipped
/// rather than returned, because handing an app id nothing can start to a
/// launcher turns a missing handler into a launch failure further away from the
/// cause.
///
/// A `[Removed Associations]` entry in a file blocks that id for every file from
/// there on, including its own. So a user's own list can veto a system default,
/// which is the direction the precedence order exists for.
pub fn default_handler(
    files: &[MimeApps],
    mime: &str,
    installed: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut blocked: HashSet<&str> = HashSet::new();
    for file in files {
        for id in file.ids(Group::Removed, mime) {
            blocked.insert(id.as_str());
        }
        for id in file.ids(Group::Default, mime) {
            if !blocked.contains(id.as_str()) && installed(id) {
                return Some(id.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn a_default_is_read_from_its_group() {
        let f = parse("[Default Applications]\nimage/png=org.arlen.Viewer.desktop;\n");
        assert_eq!(
            default_handler(&[f], "image/png", all).as_deref(),
            Some("org.arlen.Viewer.desktop")
        );
    }

    #[test]
    fn a_type_with_no_entry_resolves_to_nothing() {
        let f = parse("[Default Applications]\nimage/png=v.desktop;\n");
        assert_eq!(default_handler(&[f], "text/plain", all), None);
    }

    /// The whole point of the precedence order: the user's file is asked first.
    #[test]
    fn an_earlier_file_wins_over_a_later_one() {
        let user = parse("[Default Applications]\ntext/plain=mine.desktop;\n");
        let system = parse("[Default Applications]\ntext/plain=theirs.desktop;\n");
        assert_eq!(
            default_handler(&[user, system], "text/plain", all).as_deref(),
            Some("mine.desktop")
        );
    }

    /// A file that says nothing about the type does not stop the search.
    #[test]
    fn a_silent_earlier_file_falls_through() {
        let user = parse("[Default Applications]\nimage/png=v.desktop;\n");
        let system = parse("[Default Applications]\ntext/plain=ed.desktop;\n");
        assert_eq!(
            default_handler(&[user, system], "text/plain", all).as_deref(),
            Some("ed.desktop")
        );
    }

    /// Returning an app id nothing can start would move the failure away from
    /// its cause, so an uninstalled default is skipped like any other miss.
    #[test]
    fn an_uninstalled_default_is_skipped_for_the_next_candidate() {
        let f = parse("[Default Applications]\ntext/plain=gone.desktop;here.desktop;\n");
        assert_eq!(
            default_handler(&[f], "text/plain", |id| id == "here.desktop").as_deref(),
            Some("here.desktop")
        );
    }

    #[test]
    fn every_candidate_uninstalled_resolves_to_nothing() {
        let f = parse("[Default Applications]\ntext/plain=a.desktop;b.desktop;\n");
        assert_eq!(default_handler(&[f], "text/plain", |_| false), None);
    }

    /// The user vetoing a system default is the direction precedence exists for.
    #[test]
    fn a_removal_in_an_earlier_file_blocks_a_later_default() {
        let user = parse("[Removed Associations]\ntext/plain=theirs.desktop;\n");
        let system = parse("[Default Applications]\ntext/plain=theirs.desktop;ok.desktop;\n");
        assert_eq!(
            default_handler(&[user, system], "text/plain", all).as_deref(),
            Some("ok.desktop")
        );
    }

    /// A removal alongside the default it removes, in one file, still applies -
    /// otherwise a file could not correct itself.
    #[test]
    fn a_removal_applies_within_its_own_file() {
        let f = parse(
            "[Default Applications]\ntext/plain=no.desktop;yes.desktop;\n\
             [Removed Associations]\ntext/plain=no.desktop;\n",
        );
        assert_eq!(
            default_handler(&[f], "text/plain", all).as_deref(),
            Some("yes.desktop")
        );
    }

    /// Added associations answer "what could open this", which is a picker's
    /// question. A launch must not silently pick one of them.
    #[test]
    fn an_added_association_is_never_launched_as_a_default() {
        let f = parse("[Added Associations]\ntext/plain=maybe.desktop;\n");
        assert_eq!(
            default_handler(std::slice::from_ref(&f), "text/plain", all),
            None
        );
        assert_eq!(f.added[0].1, vec!["maybe.desktop"]);
    }

    #[test]
    fn comments_blank_lines_and_unknown_groups_are_skipped() {
        let f = parse(
            "# a comment\n\n[Nonsense]\ntext/plain=wrong.desktop;\n\
             [Default Applications]\n# another\ntext/plain=right.desktop;\n",
        );
        assert_eq!(
            default_handler(&[f], "text/plain", all).as_deref(),
            Some("right.desktop")
        );
    }

    /// One malformed line must not cost the user every other handler.
    #[test]
    fn a_line_without_a_separator_does_not_lose_the_file() {
        let f = parse("[Default Applications]\nnonsense\ntext/plain=ok.desktop;\n");
        assert_eq!(
            default_handler(&[f], "text/plain", all).as_deref(),
            Some("ok.desktop")
        );
    }

    #[test]
    fn a_repeated_type_in_one_group_keeps_the_first() {
        let f = parse(
            "[Default Applications]\ntext/plain=first.desktop;\ntext/plain=second.desktop;\n",
        );
        assert_eq!(
            default_handler(&[f], "text/plain", all).as_deref(),
            Some("first.desktop")
        );
    }

    #[test]
    fn an_empty_value_is_not_a_handler() {
        let f = parse("[Default Applications]\ntext/plain=\ntext/html=ok.desktop;\n");
        assert_eq!(
            default_handler(std::slice::from_ref(&f), "text/plain", all),
            None
        );
        assert_eq!(
            default_handler(&[f], "text/html", all).as_deref(),
            Some("ok.desktop")
        );
    }

    #[test]
    fn surrounding_whitespace_is_not_part_of_the_names() {
        let f = parse("[Default Applications]\n  text/plain = a.desktop ; b.desktop ;\n");
        assert_eq!(
            default_handler(&[f], "text/plain", |id| id == "b.desktop").as_deref(),
            Some("b.desktop")
        );
    }

    #[test]
    fn no_files_at_all_resolves_to_nothing() {
        assert_eq!(default_handler(&[], "text/plain", all), None);
    }
}
