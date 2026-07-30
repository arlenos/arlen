//! What a consented reader may see of a terminal's blocks (`tier-c-gaps-plan.md`,
//! the Terminal MCP read half).
//!
//! The run half lets the assistant execute; this is the other direction, letting
//! it SEE a terminal's recent output without running anything - the "why did that
//! fail" case. Seeing is not free: a terminal carries whatever the user typed,
//! including things they would never hand an assistant, so the read is scoped
//! rather than open.
//!
//! This module is the pure decision. It holds no blocks, opens no socket and
//! knows no caller identity; it answers "given these blocks and this consented
//! scope, which ones are visible, in what order, how many". The socket, the
//! capability gate and the consent prompt live at the edges and are tested
//! separately.
//!
//! Two rules are structural rather than configurable.
//!
//! **Never a bulk read.** A request cannot ask for more than [`MAX_BLOCKS`]
//! however large its limit, so a consented read stays a look at recent activity
//! and never becomes an export of the session.
//!
//! **The user's commands and the assistant's own are different.** A block the
//! assistant ran ([`Origin::Agent`]) is output it caused, and reading it back is
//! unremarkable. A block the user typed ([`Origin::You`]) is theirs, and only a
//! scope that says so exposes it. So the default reveals the assistant's own
//! trail and nothing else, and widening it is a decision the consent surface
//! makes explicit rather than a flag that quietly defaults open.

use crate::{Block, Origin};

/// The most blocks any single read may return, whatever it asks for. A look at
/// recent activity, never an export of the session.
pub const MAX_BLOCKS: usize = 20;

/// What a consented read may see of ONE terminal's blocks.
///
/// Per-terminal scoping is the caller's: it holds the blocks per terminal and
/// passes the one the consent names, because a `Block` carries no terminal id of
/// its own. Everything below that is decided here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadScope {
    /// How many blocks to return, newest first. Clamped to [`MAX_BLOCKS`].
    pub limit: usize,
    /// Whether the user's own typed commands are included. `false` reveals only
    /// what the assistant itself ran.
    pub include_user_blocks: bool,
    /// Whether a still-running block (no exit code yet) is included. Its output
    /// is partial and will change, so a reader that wants a settled answer says
    /// no.
    pub include_running: bool,
}

impl Default for ReadScope {
    /// The narrowest useful read: the assistant's own finished blocks, a few of
    /// them. Anything wider is a decision someone made.
    fn default() -> Self {
        Self { limit: 5, include_user_blocks: false, include_running: false }
    }
}

/// Whether one block is visible under `scope`, ignoring ordering and the cap.
///
/// Split out from [`select`] so the per-block rule is testable on its own and
/// stays readable: a change to what "visible" means is a change to this function
/// alone.
pub fn is_visible(block: &Block, scope: &ReadScope) -> bool {
    if !scope.include_user_blocks && block.origin == Origin::You {
        return false;
    }
    if !scope.include_running && block.exit_code.is_none() {
        return false;
    }
    true
}

/// The blocks a consented read returns: newest first, visible under `scope`, and
/// never more than [`MAX_BLOCKS`].
///
/// `blocks` is one terminal's, in the order they ran (oldest first), which is how
/// the terminal keeps them. The result is reversed because "recent output" is the
/// question being asked, and truncated after filtering so a run of hidden blocks
/// cannot silently eat the caller's budget.
pub fn select<'a>(blocks: &'a [Block], scope: &ReadScope) -> Vec<&'a Block> {
    let cap = scope.limit.min(MAX_BLOCKS);
    blocks
        .iter()
        .rev()
        .filter(|b| is_visible(b, scope))
        .take(cap)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockBodyKind;

    fn block(id: &str, origin: Origin, exit: Option<i32>) -> Block {
        Block {
            id: id.to_string(),
            command: format!("cmd-{id}"),
            exit_code: exit,
            duration_ms: exit.map(|_| 1),
            cwd: "/w".to_string(),
            git: None,
            origin,
            body_kind: BlockBodyKind::Grid,
            body: serde_json::Value::Null,
        }
    }

    #[test]
    fn the_default_scope_shows_only_the_assistants_own_finished_blocks() {
        let blocks = vec![
            block("a", Origin::You, Some(0)),
            block("b", Origin::Agent, Some(0)),
            block("c", Origin::Agent, None),
        ];
        let got: Vec<&str> = select(&blocks, &ReadScope::default())
            .iter()
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(got, ["b"], "the user's block and the running one must not leak");
    }

    #[test]
    fn user_blocks_appear_only_when_the_scope_says_so() {
        let blocks = vec![block("a", Origin::You, Some(0))];
        assert!(select(&blocks, &ReadScope::default()).is_empty());
        let widened = ReadScope { include_user_blocks: true, ..ReadScope::default() };
        assert_eq!(select(&blocks, &widened).len(), 1);
    }

    #[test]
    fn a_running_block_appears_only_when_the_scope_says_so() {
        let blocks = vec![block("a", Origin::Agent, None)];
        assert!(select(&blocks, &ReadScope::default()).is_empty());
        let widened = ReadScope { include_running: true, ..ReadScope::default() };
        assert_eq!(select(&blocks, &widened).len(), 1);
    }

    #[test]
    fn the_newest_blocks_come_first() {
        let blocks: Vec<Block> = ["a", "b", "c"]
            .iter()
            .map(|i| block(i, Origin::Agent, Some(0)))
            .collect();
        let got: Vec<&str> = select(&blocks, &ReadScope { limit: 3, ..ReadScope::default() })
            .iter()
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(got, ["c", "b", "a"]);
    }

    #[test]
    fn no_request_can_read_more_than_the_cap() {
        // The invariant is the cap, not the requested number: asking for a
        // thousand must not turn a look into an export.
        let blocks: Vec<Block> = (0..100)
            .map(|i| block(&i.to_string(), Origin::Agent, Some(0)))
            .collect();
        let huge = ReadScope { limit: 1000, ..ReadScope::default() };
        assert_eq!(select(&blocks, &huge).len(), MAX_BLOCKS);
    }

    #[test]
    fn hidden_blocks_do_not_consume_the_limit() {
        // Ten of the user's blocks in front of two of the assistant's must not
        // return an empty answer: filtering happens before the take, so the
        // caller gets the two it may see.
        let mut blocks: Vec<Block> =
            (0..10).map(|i| block(&format!("u{i}"), Origin::You, Some(0))).collect();
        blocks.insert(0, block("a1", Origin::Agent, Some(0)));
        blocks.insert(0, block("a2", Origin::Agent, Some(0)));
        let got = select(&blocks, &ReadScope { limit: 5, ..ReadScope::default() });
        assert_eq!(got.len(), 2);
    }
}
