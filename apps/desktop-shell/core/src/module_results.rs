//! SX-4: letting sandboxed modules contribute launcher results, safely.
//!
//! A Tier-1 module runs in a WASM sandbox with a declared capability set, and
//! the whole point of that sandbox is that the module cannot reach the system
//! except through checked host calls. A search result is the one thing it hands
//! back that the SHELL then acts on, which makes it the seam where that
//! containment can be undone by accident.
//!
//! The protocol's `SearchAction` includes `Execute { command }`, because it is
//! also the shape first-party in-process plugins use, and for them running a
//! command is fine - they are the shell. For a module it is not: honouring it
//! would let a sandboxed module have the shell run an arbitrary command
//! unconfined, which is a more direct path to the system than any host call it
//! was ever granted.
//!
//! So the mapping is a filter, not a conversion. What a module may ask for is
//! bounded here, and a result it cannot be trusted with is dropped rather than
//! rendered as an entry that would fail or, worse, succeed.

use modulesd_proto::{SearchAction, SearchResult as ModuleResult};

/// Why a module's result was not offered to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// The action would have the shell run a command on the module's behalf.
    WouldExecute,
    /// The action names a handler the shell does not implement, so the entry
    /// could only ever fail when clicked.
    UnknownHandler,
    /// The title was empty, which renders as an unlabelled row the user cannot
    /// judge before clicking.
    Unlabelled,
}

/// A module result the shell is willing to show.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleEntry {
    /// The module that produced it, so the row can say where it came from.
    pub module_id: String,
    /// The result's own id, needed to send an execute back to the module.
    pub id: String,
    /// What to show.
    pub title: String,
    /// Optional second line.
    pub description: Option<String>,
    /// How the shell should act on it.
    pub action: SafeAction,
}

/// The actions a module may ask the shell to take.
///
/// A closed set, deliberately smaller than the protocol's: adding a variant is
/// a decision about what a sandboxed module may reach, and having to write it
/// here is the point at which that decision gets made rather than inherited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeAction {
    /// Put text on the clipboard.
    Copy(String),
    /// Open a URL through the normal handler.
    OpenUrl(String),
    /// Open a path through the normal handler.
    OpenPath(String),
}

/// Accept the module results the shell can safely offer, dropping the rest.
///
/// Returns the accepted entries and, separately, what was rejected and why -
/// a dropped result is worth logging, since a module whose entries never appear
/// is otherwise indistinguishable from a module that found nothing.
pub fn accept(results: Vec<ModuleResult>) -> (Vec<ModuleEntry>, Vec<(String, Rejected)>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();

    for r in results {
        if r.title.trim().is_empty() {
            dropped.push((r.id, Rejected::Unlabelled));
            continue;
        }
        let action = match r.action {
            SearchAction::Copy { text } => SafeAction::Copy(text),
            SearchAction::OpenUrl { url } => SafeAction::OpenUrl(url),
            SearchAction::OpenPath { path } => SafeAction::OpenPath(path),
            // The one that would undo the sandbox.
            SearchAction::Execute { .. } => {
                dropped.push((r.id, Rejected::WouldExecute));
                continue;
            }
            // A module naming a shell-internal handler is either mistaken or
            // probing for one; either way the shell implements none for
            // modules, so the entry could only fail.
            SearchAction::Custom { .. } => {
                dropped.push((r.id, Rejected::UnknownHandler));
                continue;
            }
        };
        kept.push(ModuleEntry {
            module_id: r.plugin_id,
            id: r.id,
            title: r.title,
            description: r.description,
            action,
        });
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: &str, action: SearchAction) -> ModuleResult {
        ModuleResult {
            id: id.into(),
            title: "A result".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action,
            plugin_id: "com.example.weather".into(),
        }
    }

    /// The whole reason this filter exists: a sandboxed module must not get the
    /// shell to run a command for it, which would be a more direct path to the
    /// system than any host call it was granted.
    #[test]
    fn a_module_cannot_have_the_shell_run_a_command() {
        let (kept, dropped) = accept(vec![result(
            "r1",
            SearchAction::Execute {
                command: "rm -rf ~".into(),
            },
        )]);
        assert!(kept.is_empty());
        assert_eq!(dropped, vec![("r1".to_string(), Rejected::WouldExecute)]);
    }

    #[test]
    fn the_bounded_actions_are_carried_through() {
        let (kept, dropped) = accept(vec![
            result("a", SearchAction::Copy { text: "x".into() }),
            result(
                "b",
                SearchAction::OpenUrl {
                    url: "https://example.com".into(),
                },
            ),
            result(
                "c",
                SearchAction::OpenPath {
                    path: "/tmp/x".into(),
                },
            ),
        ]);
        assert!(dropped.is_empty());
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].action, SafeAction::Copy("x".into()));
        assert_eq!(kept[0].module_id, "com.example.weather");
    }

    /// A handler the shell does not implement can only fail when clicked.
    #[test]
    fn a_custom_handler_is_not_offered() {
        let (kept, dropped) = accept(vec![result(
            "r1",
            SearchAction::Custom {
                handler: "core.power".into(),
                data: "shutdown".into(),
            },
        )]);
        assert!(kept.is_empty());
        assert_eq!(dropped[0].1, Rejected::UnknownHandler);
    }

    /// An unlabelled row is one the user cannot judge before clicking.
    #[test]
    fn an_untitled_result_is_not_offered() {
        let mut r = result("r1", SearchAction::Copy { text: "x".into() });
        r.title = "   ".into();
        let (kept, dropped) = accept(vec![r]);
        assert!(kept.is_empty());
        assert_eq!(dropped[0].1, Rejected::Unlabelled);
    }

    /// One bad result must not cost the module its good ones.
    #[test]
    fn a_rejected_result_does_not_drop_its_siblings() {
        let (kept, dropped) = accept(vec![
            result("good", SearchAction::Copy { text: "x".into() }),
            result(
                "bad",
                SearchAction::Execute {
                    command: "x".into(),
                },
            ),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "good");
        assert_eq!(dropped.len(), 1);
    }
}
