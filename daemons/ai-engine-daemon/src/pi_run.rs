//! The ephemeral pi run (`pi-agent-adoption.md` §D/§E): for a `kind: agent`
//! behaviour, the autonomous curator spawns a BOUNDED, headless, bwrap-confined
//! pi process for ONE trigger, drives it through the gated contract, and tears it
//! down - distinct from the persistent interactive supervisor. This module builds
//! the per-trigger session; the spawn + teardown is a later increment.

use ai_engine_contract::{CapabilityContext, ReadTier, SessionInit};
use arlen_ai_skills::behaviour::{Behaviour, ReadScope};

/// Map a behaviour's declared [`ReadScope`] to the contract [`ReadTier`] the
/// session reads under. Both are five-level and ordinally aligned, so this is the
/// order-preserving correspondence (no graph -> no read, ... full -> full). The
/// graph compiler enforces the resulting tier + its active-project anchor.
pub fn read_tier_for_scope(scope: ReadScope) -> ReadTier {
    match scope {
        ReadScope::Minimal => ReadTier::None,
        ReadScope::Session => ReadTier::Minimal,
        ReadScope::Project => ReadTier::Standard,
        ReadScope::Time => ReadTier::Extended,
        ReadScope::Full => ReadTier::Full,
    }
}

/// Whether a declared tool name is a PRIVILEGED proxy tool - one the daemon runs
/// in trusted Rust via `Execute` (KG + OS mutations) - vs a generic in-engine
/// tool. Used only to split the SessionInit's coarse capability context; the gate
/// enforces every call regardless of this classification.
pub fn is_privileged_proxy_tool(tool: &str) -> bool {
    tool.starts_with("graph.") || tool.starts_with("fs.") || tool.starts_with("os.")
}

/// Build the [`SessionInit`] for an ephemeral autonomous pi run of `behaviour`.
///
/// The system prompt is the behaviour's body (the skill instructions); the
/// capability context lists the behaviour's declared tools split into generic vs
/// privileged-proxy; the read tier comes from the behaviour's declared read scope;
/// and `externally_triggered` is TRUE - an autonomous-curator run is started by an
/// event (external origin, HIGH-2), so the gate escalates every action to a
/// confirmation unless the deterministic-workflow carve-out applies (which it does
/// NOT for a `kind: agent` run, so an agent's mutating action always confirms).
///
/// NOTE (§F2, defense-in-depth follow-up): the capability context is PROMPT
/// CONTEXT ONLY - the gate is the real per-call authority - so a behaviour cannot
/// escalate by over-declaring tools here. Supplying a CURATED least-authority tool
/// set (rather than the behaviour's self-declared list) is the §F2 hardening, not
/// yet wired.
pub fn build_ephemeral_session_init(
    behaviour: &Behaviour,
    project_anchor: Option<String>,
) -> SessionInit {
    let (proxy_tools, generic_tools): (Vec<String>, Vec<String>) = behaviour
        .manifest
        .tools
        .keys()
        .cloned()
        .partition(|t| is_privileged_proxy_tool(t));
    SessionInit {
        system_prompt: behaviour.body.clone(),
        behaviour: Some(behaviour.manifest.name.clone()),
        capability_context: CapabilityContext { generic_tools, proxy_tools },
        project_anchor,
        read_tier: read_tier_for_scope(behaviour.manifest.reads),
        externally_triggered: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_behaviour(name: &str) -> Behaviour {
        // A complete agent SKILL.md declaring one privileged + one generic tool.
        let src = format!(
            "---\nname: {name}\ndescription: d\nkind: agent\nreads: project\nmode: suggest\n\
             trigger:\n  type: event\n  event: calendar.event.upcoming\ntools:\n  graph.query: []\n  \
             web.search: []\nbudget:\n  max_steps: 10\n  max_tokens: 12000\n  max_wall_ms: 15000\n\
             terminal:\n  done: silent\n---\nGather related notes.\n"
        );
        arlen_ai_skills::behaviour::parse(&src).expect("valid agent SKILL.md")
    }

    #[test]
    fn read_tier_for_scope_is_the_ordinal_alignment() {
        assert_eq!(read_tier_for_scope(ReadScope::Minimal), ReadTier::None);
        assert_eq!(read_tier_for_scope(ReadScope::Session), ReadTier::Minimal);
        assert_eq!(read_tier_for_scope(ReadScope::Project), ReadTier::Standard);
        assert_eq!(read_tier_for_scope(ReadScope::Time), ReadTier::Extended);
        assert_eq!(read_tier_for_scope(ReadScope::Full), ReadTier::Full);
    }

    #[test]
    fn privileged_proxy_classification() {
        assert!(is_privileged_proxy_tool("graph.read"));
        assert!(is_privileged_proxy_tool("fs.move"));
        assert!(is_privileged_proxy_tool("os.notify"));
        assert!(!is_privileged_proxy_tool("web.search"));
        assert!(!is_privileged_proxy_tool("bash"));
    }

    #[test]
    fn build_session_init_carries_body_tools_tier_and_external() {
        let b = agent_behaviour("meeting-prep");
        let init = build_ephemeral_session_init(&b, Some("proj-1".to_string()));
        // The body is the verbatim skill instructions (trailing newline kept).
        assert_eq!(init.system_prompt.trim(), "Gather related notes.");
        assert_eq!(init.behaviour, Some("meeting-prep".to_string()));
        // Tools split by privilege.
        assert_eq!(init.capability_context.proxy_tools, vec!["graph.query".to_string()]);
        assert_eq!(init.capability_context.generic_tools, vec!["web.search".to_string()]);
        // reads: project -> the project (standard) tier; anchored.
        assert_eq!(init.read_tier, ReadTier::Standard);
        assert_eq!(init.project_anchor, Some("proj-1".to_string()));
        // An autonomous-curator run is externally triggered (HIGH-2).
        assert!(init.externally_triggered);
    }
}
