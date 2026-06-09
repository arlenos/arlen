//! The durable undo-log's event-sourced lifecycle state machine
//! (reversible-receipts-and-the-effect-model.md §2, §6).
//!
//! An append-only, HMAC-chained log cannot flip an entry's state in place, so a
//! state transition is a new appended (chained) record, and the current state of
//! an `op_id` is the **fold** of its records on read. This module is the pure
//! state machine: the lifecycle states, their legal transitions, and the fold
//! that reconstructs the current state, rejecting an illegal sequence so a
//! corrupt or forged log fails closed rather than yielding a bogus state.
//!
//! The store itself (the fsync write-ahead append under
//! `~/.local/state/arlen/agent/undo-log/`, the HMAC chain, the access control,
//! retention by undo-window), and the separate-uid signer helper that seals it,
//! are later EM-R1 increments built on this core.

/// The lifecycle of an undo-log entry (§2). An entry is created [`InFlight`] (the
/// provisional record appended and fsynced *before* the externalised act,
/// carrying the captured inverse), then transitions as the act commits, aborts,
/// is compensated, or is superseded.
///
/// [`InFlight`]: UndoState::InFlight
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoState {
    /// Provisional: written ahead of the act. A crash leaves the entry here for
    /// the reconciler to resolve against reality (§2).
    InFlight,
    /// The act committed; the entry is undoable within the undo window.
    Committed,
    /// The act did not commit (a definite failure); there is nothing to undo.
    Aborted,
    /// Compensation (undo) has started but is not yet confirmed.
    Compensating,
    /// Compensation completed; the action was undone.
    Compensated,
    /// A user change to the same target superseded the agent's entry
    /// (the user-subsumes-agent rule §2), rather than the agent fighting it.
    Superseded,
}

impl UndoState {
    /// Whether this is a terminal state, from which no further transition is
    /// legal: the entry's lifecycle has ended.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            UndoState::Aborted | UndoState::Compensated | UndoState::Superseded
        )
    }

    /// Whether a transition from `self` to `next` is legal (§6). A user
    /// supersession may interrupt any non-terminal entry; otherwise the only
    /// paths are `InFlight -> Committed | Aborted`, `Committed -> Compensating`,
    /// and `Compensating -> Compensated`. A terminal state admits nothing.
    pub fn can_transition_to(self, next: UndoState) -> bool {
        if self.is_terminal() {
            return false;
        }
        // User-subsumes-agent: a user change to the target supersedes any
        // non-terminal entry rather than being fought.
        if next == UndoState::Superseded {
            return true;
        }
        matches!(
            (self, next),
            (UndoState::InFlight, UndoState::Committed)
                | (UndoState::InFlight, UndoState::Aborted)
                | (UndoState::Committed, UndoState::Compensating)
                | (UndoState::Compensating, UndoState::Compensated)
        )
    }
}

/// Fold a sequence of an entry's appended lifecycle records into its current
/// state (§6: "the current state of an `op_id` is the fold of its records on
/// read"). The first record must be [`UndoState::InFlight`] (an entry is created
/// provisional); each subsequent record must be a legal transition from the
/// running state. Returns the final state, or `Err` describing the first illegal
/// record, so a corrupt or forged record chain fails closed.
pub fn fold_state(records: &[UndoState]) -> Result<UndoState, String> {
    let mut iter = records.iter().copied();
    let Some(first) = iter.next() else {
        return Err("empty record chain: an entry must begin InFlight".to_string());
    };
    if first != UndoState::InFlight {
        return Err(format!("first record must be InFlight, was {first:?}"));
    }
    let mut current = first;
    for next in iter {
        if !current.can_transition_to(next) {
            return Err(format!("illegal transition {current:?} -> {next:?}"));
        }
        current = next;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use UndoState::*;

    #[test]
    fn a_committed_then_compensated_chain_folds_to_compensated() {
        assert_eq!(
            fold_state(&[InFlight, Committed, Compensating, Compensated]).unwrap(),
            Compensated
        );
    }

    #[test]
    fn an_aborted_chain_folds_to_aborted() {
        assert_eq!(fold_state(&[InFlight, Aborted]).unwrap(), Aborted);
    }

    #[test]
    fn a_supersession_can_interrupt_any_non_terminal_state() {
        assert_eq!(fold_state(&[InFlight, Superseded]).unwrap(), Superseded);
        assert_eq!(fold_state(&[InFlight, Committed, Superseded]).unwrap(), Superseded);
        assert_eq!(
            fold_state(&[InFlight, Committed, Compensating, Superseded]).unwrap(),
            Superseded
        );
    }

    #[test]
    fn an_empty_or_non_inflight_start_is_rejected() {
        assert!(fold_state(&[]).is_err());
        assert!(fold_state(&[Committed]).is_err(), "an entry must begin InFlight");
    }

    #[test]
    fn an_illegal_transition_fails_closed() {
        // InFlight cannot jump straight to Compensated (no commit, no compensate).
        assert!(fold_state(&[InFlight, Compensated]).is_err());
        // Committed cannot go back to InFlight.
        assert!(fold_state(&[InFlight, Committed, InFlight]).is_err());
        // A terminal state admits nothing further.
        assert!(fold_state(&[InFlight, Aborted, Committed]).is_err());
        assert!(fold_state(&[InFlight, Superseded, Committed]).is_err());
    }

    #[test]
    fn terminal_states_admit_no_transition() {
        for terminal in [Aborted, Compensated, Superseded] {
            assert!(terminal.is_terminal());
            for next in [InFlight, Committed, Aborted, Compensating, Compensated, Superseded] {
                assert!(!terminal.can_transition_to(next), "{terminal:?} -> {next:?} must be illegal");
            }
        }
    }
}
