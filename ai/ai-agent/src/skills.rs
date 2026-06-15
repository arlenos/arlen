//! User-invoke + agent-match over the loaded skills (PR-5 part 3).
//!
//! A behaviour (`SKILL.md`) has one format and three entry points
//! (`ai-agent-design.md` §3): event-triggered (the dispatcher), user-invocable
//! (the harness lists loaded skills and runs one), and agent-matched (the daemon
//! checks a free-form task against each skill's `whenToUse` before falling back
//! to a plain answer). This module is the backend for the latter two over the
//! *same* loaded set: a serialisable [`SkillSummary`] list and a deterministic,
//! model-free [`match_skill`]. The harness surface (the Tauri command that calls
//! [`skill_summaries`]) and the run-through-the-loop wiring are the consumers.

use serde::Serialize;

use crate::behaviour::BehaviourKind;
use crate::loader::{LoadedBehaviour, Status};

/// A loaded skill as the user-invoke list shows it: identity + the routing
/// hints, never the body. Serialisable for the discovery command.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillSummary {
    /// Stable kebab-case name (the invoke key).
    pub name: String,
    /// One-line description.
    pub description: String,
    /// The agent-match hint, if the skill declares one.
    pub when_to_use: Option<String>,
    /// Workflow or agent.
    pub kind: BehaviourKind,
    /// Whether it is enabled (trusted Settings state); a disabled skill is
    /// listed but cannot run.
    pub enabled: bool,
}

/// Summarise every loaded skill for the user-invoke list (the deferred S-U3b
/// discovery command). Order follows the loaded order, which the loader has
/// already de-duplicated by name.
pub fn skill_summaries(loaded: &[LoadedBehaviour]) -> Vec<SkillSummary> {
    loaded
        .iter()
        .map(|lb| {
            let m = &lb.behaviour.manifest;
            SkillSummary {
                name: m.name.clone(),
                description: m.description.clone(),
                when_to_use: m.when_to_use.clone(),
                kind: m.kind,
                enabled: lb.status == Status::Enabled,
            }
        })
        .collect()
}

/// Significant word tokens of a free-form string: lowercased ASCII-alphanumeric
/// runs of at least three characters, deduplicated. The length floor drops
/// trivial connectives ("a", "to", "of") so the overlap reflects topical words,
/// not grammar.
fn significant_tokens(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// Agent-match a free-form task against the loaded skills' `whenToUse` hints.
///
/// Deterministic and model-free: the best skill is the ENABLED one whose
/// `whenToUse` shares the most significant word tokens with the task. A skill
/// with no `whenToUse`, a disabled skill, or zero shared tokens never matches;
/// no match (`None`) is the signal to fall back to a plain answer. Ties are
/// broken by name order so the result is stable. This is a cheap prefilter, not
/// a ranking model — it only decides *whether* a skill is plausibly relevant
/// before the daemon commits the task to the bounded tool loop.
pub fn match_skill<'a>(task: &str, loaded: &'a [LoadedBehaviour]) -> Option<&'a LoadedBehaviour> {
    let task_tokens = significant_tokens(task);
    if task_tokens.is_empty() {
        return None;
    }

    let mut best: Option<(&LoadedBehaviour, usize)> = None;
    for lb in loaded {
        if lb.status != Status::Enabled {
            continue;
        }
        let Some(hint) = &lb.behaviour.manifest.when_to_use else {
            continue;
        };
        let overlap = significant_tokens(hint).intersection(&task_tokens).count();
        if overlap == 0 {
            continue;
        }
        let better = match best {
            None => true,
            // Strictly more overlap wins; an equal overlap keeps the
            // earlier-by-name skill so the choice is deterministic.
            Some((cur, cur_overlap)) => {
                overlap > cur_overlap
                    || (overlap == cur_overlap
                        && lb.behaviour.manifest.name < cur.behaviour.manifest.name)
            }
        };
        if better {
            best = Some((lb, overlap));
        }
    }
    best.map(|(lb, _)| lb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{DisableReason, Provenance};
    use std::path::PathBuf;

    fn disabled() -> Status {
        Status::Disabled(DisableReason::NotEnabledInSettings)
    }

    fn loaded(md: &str, status: Status) -> LoadedBehaviour {
        let behaviour = crate::behaviour::parse(md).expect("fixture parses");
        LoadedBehaviour {
            behaviour,
            provenance: Provenance::BuiltIn,
            dir: PathBuf::from("/x"),
            status,
        }
    }

    fn skill(name: &str, when_to_use: Option<&str>, status: Status) -> LoadedBehaviour {
        let hint = match when_to_use {
            Some(w) => format!("whenToUse: \"{w}\"\n"),
            None => String::new(),
        };
        let md = format!(
            "---\nname: {name}\ndescription: d\n{hint}kind: workflow\nhandler: h\n\
             trigger:\n  type: manual\n---\nbody\n"
        );
        loaded(&md, status)
    }

    #[test]
    fn whenToUse_parses_and_summarises() {
        let s = skill("tidy", Some("clean up the downloads folder"), Status::Enabled);
        assert_eq!(
            s.behaviour.manifest.when_to_use.as_deref(),
            Some("clean up the downloads folder")
        );
        let sum = skill_summaries(std::slice::from_ref(&s));
        assert_eq!(sum.len(), 1);
        assert_eq!(sum[0].name, "tidy");
        assert_eq!(sum[0].when_to_use.as_deref(), Some("clean up the downloads folder"));
        assert!(sum[0].enabled);
    }

    #[test]
    fn absent_whenToUse_is_none_and_never_matches() {
        // A skill with no whenToUse still loads (the field is optional) but is
        // never agent-matched.
        let s = skill("plain", None, Status::Enabled);
        assert!(s.behaviour.manifest.when_to_use.is_none());
        assert!(match_skill("clean the downloads", std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn match_picks_the_best_overlap() {
        let skills = vec![
            skill("tidy-downloads", Some("clean up the downloads folder"), Status::Enabled),
            skill("summarise-day", Some("what did I work on today"), Status::Enabled),
        ];
        let m = match_skill("please clean my downloads folder", &skills).expect("a match");
        assert_eq!(m.behaviour.manifest.name, "tidy-downloads");

        let m2 = match_skill("what did I work on", &skills).expect("a match");
        assert_eq!(m2.behaviour.manifest.name, "summarise-day");
    }

    #[test]
    fn no_overlap_returns_none() {
        let skills = vec![skill("tidy", Some("clean the downloads folder"), Status::Enabled)];
        assert!(match_skill("schedule a dentist appointment", &skills).is_none());
        // An empty / token-less task also yields no match.
        assert!(match_skill("  !! ", &skills).is_none());
    }

    #[test]
    fn disabled_skill_is_listed_but_never_matches() {
        let s = skill("tidy", Some("clean the downloads folder"), disabled());
        // Listed (so the harness can show its disabled state)...
        assert!(!skill_summaries(std::slice::from_ref(&s))[0].enabled);
        // ...but never agent-matched.
        assert!(match_skill("clean my downloads", std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn equal_overlap_breaks_ties_by_name() {
        // Both hints share exactly one token ("notes") with the task; the
        // earlier name wins, deterministically.
        let skills = vec![
            skill("zeta", Some("notes"), Status::Enabled),
            skill("alpha", Some("notes"), Status::Enabled),
        ];
        let m = match_skill("show my notes", &skills).expect("a match");
        assert_eq!(m.behaviour.manifest.name, "alpha");
    }
}
