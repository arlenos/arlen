//! Assembling warp-style command [`Block`]s from the engine's [`VtEvent`] stream
//! (terminal-ui-plan.md §3-§5, the backend half of "make the terminal run").
//!
//! A block is the FRAME around one command: its command line, exit code, timing,
//! working directory and origin. The command's OUTPUT TEXT is NOT carried here -
//! ordinary output is painted by the grid subsurface ([`BlockBodyKind::Grid`], a
//! reserved transparent hole), so a block's `body` is empty for grid output. The
//! host drains the engine's OSC-mark events ([`VtEngine::drain_events`]) and feeds
//! them here; this tracks the in-progress block and the finished ones.
//!
//! The OSC marks frame a command as: a `CommandLine` (the nonce-verified command,
//! 633;E) opens a block, `ExecStart` (133;C) starts the duration clock,
//! `CommandEnd` (133;D) closes it with its exit code, and `CwdChanged` (OSC 7 /
//! 633;P) tracks the directory new blocks inherit. Time is injected (the host
//! passes `Instant::now()` per drain), so the assembler stays pure and testable.
//!
//! [`VtEngine::drain_events`]: crate::vt::VtEngine::drain_events

use std::time::Instant;

use crate::vt::VtEvent;
use crate::{Block, BlockBodyKind, Origin};

/// The block currently being assembled: a command has been issued but its
/// `CommandEnd` has not yet arrived.
struct Pending {
    id: String,
    command: String,
    cwd: String,
    /// When `ExecStart` was observed, for the duration on `CommandEnd`.
    exec_start: Option<Instant>,
}

/// Consumes the engine's [`VtEvent`] stream and produces the block list the host
/// serves to the UI.
pub struct BlockAssembler {
    finished: Vec<Block>,
    pending: Option<Pending>,
    /// The latest working directory (from `CwdChanged`); new blocks inherit it.
    cwd: String,
    next_id: u64,
}

impl BlockAssembler {
    /// A fresh assembler seeded with the session's initial working directory.
    pub fn new(initial_cwd: impl Into<String>) -> Self {
        Self {
            finished: Vec::new(),
            pending: None,
            cwd: initial_cwd.into(),
            next_id: 0,
        }
    }

    /// Process a batch of events drained from the engine, using `now` for timing
    /// (the host passes `Instant::now()`; tests pass a controlled clock). Events
    /// must be in the order the engine produced them.
    pub fn consume(&mut self, events: &[VtEvent], now: Instant) {
        for ev in events {
            match ev {
                VtEvent::CwdChanged { cwd } => self.cwd = cwd.clone(),
                VtEvent::CommandLine { command } => {
                    // A new command opens a block. If a prior block was left open
                    // (a `CommandLine` without its `CommandEnd`), finalize it with
                    // no exit/duration so it is not lost.
                    self.finalize_dangling();
                    self.next_id += 1;
                    self.pending = Some(Pending {
                        id: format!("b{}", self.next_id),
                        command: command.clone(),
                        cwd: self.cwd.clone(),
                        exec_start: None,
                    });
                }
                VtEvent::ExecStart => {
                    if let Some(p) = self.pending.as_mut() {
                        p.exec_start = Some(now);
                    }
                }
                VtEvent::CommandEnd { exit_code } => {
                    if let Some(p) = self.pending.take() {
                        let duration_ms = p
                            .exec_start
                            .map(|s| now.saturating_duration_since(s).as_millis() as u64);
                        self.finished
                            .push(finished_block(p, *exit_code, duration_ms));
                    }
                }
                // PromptStart bounds a prompt, Title is window chrome: neither
                // opens or closes a block.
                VtEvent::PromptStart | VtEvent::Title { .. } => {}
            }
        }
    }

    /// Close a still-open block (no `CommandEnd` seen) before a new one opens.
    fn finalize_dangling(&mut self) {
        if let Some(p) = self.pending.take() {
            self.finished.push(finished_block(p, None, None));
        }
    }

    /// The assembled blocks: every finished block, plus the in-progress one (with
    /// `None` exit/duration) so the UI shows the running command live.
    pub fn blocks(&self) -> Vec<Block> {
        let mut out = self.finished.clone();
        if let Some(p) = self.pending.as_ref() {
            out.push(Block {
                id: p.id.clone(),
                command: p.command.clone(),
                exit_code: None,
                duration_ms: None,
                cwd: p.cwd.clone(),
                git: None,
                origin: Origin::You,
                body_kind: BlockBodyKind::Grid,
                body: serde_json::Value::Null,
            });
        }
        out
    }
}

/// Build a finished block frame. The body is empty: grid output is painted by the
/// subsurface, not carried in the contract.
fn finished_block(p: Pending, exit_code: Option<i32>, duration_ms: Option<u64>) -> Block {
    Block {
        id: p.id,
        command: p.command,
        exit_code,
        duration_ms,
        cwd: p.cwd,
        git: None,
        origin: Origin::You,
        body_kind: BlockBodyKind::Grid,
        body: serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_full_command_cycle_produces_one_finished_block() {
        let mut a = BlockAssembler::new("/home/u");
        let start = t0();
        a.consume(
            &[
                VtEvent::PromptStart,
                VtEvent::CommandLine {
                    command: "ls -la".to_string(),
                },
                VtEvent::ExecStart,
            ],
            start,
        );
        // While running: one in-progress block, no exit/duration.
        let live = a.blocks();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].command, "ls -la");
        assert_eq!(live[0].exit_code, None);
        assert_eq!(live[0].cwd, "/home/u");

        // The command ends 40ms later.
        a.consume(
            &[VtEvent::CommandEnd { exit_code: Some(0) }],
            start + Duration::from_millis(40),
        );
        let done = a.blocks();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].exit_code, Some(0));
        assert_eq!(done[0].duration_ms, Some(40));
    }

    #[test]
    fn cwd_change_is_inherited_by_the_next_block_not_the_running_one() {
        let mut a = BlockAssembler::new("/home/u");
        let now = t0();
        a.consume(
            &[
                VtEvent::CommandLine {
                    command: "cd /tmp".to_string(),
                },
                VtEvent::ExecStart,
                VtEvent::CwdChanged {
                    cwd: "/tmp".to_string(),
                },
                VtEvent::CommandEnd { exit_code: Some(0) },
            ],
            now,
        );
        // The `cd` block ran in the OLD cwd; the cwd change applies to the next.
        a.consume(
            &[VtEvent::CommandLine {
                command: "pwd".to_string(),
            }],
            now,
        );
        let blocks = a.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].cwd, "/home/u", "the cd block ran in the old cwd");
        assert_eq!(blocks[1].cwd, "/tmp", "the next block inherits the new cwd");
    }

    #[test]
    fn a_dangling_command_is_finalized_when_the_next_opens() {
        let mut a = BlockAssembler::new("/home/u");
        let now = t0();
        // Two CommandLines with no CommandEnd between them (a missed close).
        a.consume(
            &[
                VtEvent::CommandLine {
                    command: "first".to_string(),
                },
                VtEvent::CommandLine {
                    command: "second".to_string(),
                },
            ],
            now,
        );
        let blocks = a.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].command, "first");
        assert_eq!(blocks[0].exit_code, None, "the dangling block has no exit");
        assert_eq!(blocks[1].command, "second");
    }

    #[test]
    fn a_failing_command_records_its_exit_code() {
        let mut a = BlockAssembler::new("/work");
        let now = t0();
        a.consume(
            &[
                VtEvent::CommandLine {
                    command: "false".to_string(),
                },
                VtEvent::ExecStart,
                VtEvent::CommandEnd { exit_code: Some(1) },
            ],
            now,
        );
        let blocks = a.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].exit_code, Some(1));
        assert_eq!(blocks[0].body_kind, BlockBodyKind::Grid);
    }

    #[test]
    fn ids_are_stable_and_unique_across_blocks() {
        let mut a = BlockAssembler::new("/");
        let now = t0();
        for cmd in ["a", "b", "c"] {
            a.consume(
                &[
                    VtEvent::CommandLine {
                        command: cmd.to_string(),
                    },
                    VtEvent::CommandEnd { exit_code: Some(0) },
                ],
                now,
            );
        }
        let ids: Vec<_> = a.blocks().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["b1", "b2", "b3"]);
    }
}
