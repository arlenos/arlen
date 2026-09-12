//! Agent-match over the loaded skills (PR-5 part 3).
//!
//! A behaviour (`SKILL.md`) has one format and three entry points
//! (`ai-agent-design.md` §3): event-triggered (the dispatcher), user-invocable
//! (the harness lists loaded skills and runs one), and agent-matched (the daemon
//! checks a free-form task against each skill's `whenToUse` before falling back
//! to a plain answer). This module is the deterministic, model-free prefilter for
//! the third.
//!
//! ## The consumer is not built, and that is a decision rather than an omission
//!
//! Measured 13 September: nothing in the tree calls [`match_skill`]. The obvious
//! call site is `ai-engine-daemon`'s `ask`, which today runs the `ask` skill for
//! every question. Routing a question to a DIFFERENT skill because their words
//! overlap would change what the run may read - the gate enforces the matched
//! skill's declared scope, not `ask`'s - and a question can carry text somebody
//! else wrote. So the match is built and the routing is a read-scope ruling,
//! which is the planner's.
//!
//! **A list surface is not what is missing.** This module also carried a
//! `SkillSummary` for "the harness lists loaded skills"; by the time anybody
//! looked, that list had been built twice elsewhere and richer - the harness's
//! own `BehaviourStatus` and Settings' AI page both carry provenance, enablement
//! reason and declared reads, which the summary here did not. It was removed
//! rather than left as a thinner third answer to a question already answered.

use crate::loader::{LoadedBehaviour, Status};

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
    fn when_to_use_parses_off_the_manifest() {
        let s = skill("tidy", Some("clean up the downloads folder"), Status::Enabled);
        assert_eq!(
            s.behaviour.manifest.when_to_use.as_deref(),
            Some("clean up the downloads folder")
        );
    }

    #[test]
    fn absent_when_to_use_is_none_and_never_matches() {
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
    fn a_disabled_skill_never_matches() {
        // It still loads - Settings lists it with its disabled reason - but a
        // task never routes to something the person has switched off.
        let s = skill("tidy", Some("clean the downloads folder"), disabled());
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
