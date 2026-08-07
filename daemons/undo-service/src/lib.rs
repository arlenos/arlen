// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading and reversing what happened on this machine.
//!
//! **Undo is not an AI feature.** These three modules were inside
//! `ai-engine-daemon`, which serves its D-Bus surface only when `[ai] enabled` is
//! true - so turning the assistant off in Settings took the history of a user's
//! own file moves and setting changes with it, and their reversal too. The
//! records never depended on the assistant: the signed undo log is its own daemon
//! under its own uid, and the audit ledger is another. Only the surface did.
//!
//! So the rule this crate exists to keep, and the one to check any change here
//! against: **nothing in it may read whether the AI is running.** Past actions
//! stay visible and stay undoable while the assistant is off - the records exist,
//! the actions happened, and switching the thing off must not erase the account
//! of what it did.
//!
//! - [`undo_signer`] talks to the signing daemon that holds the log.
//! - [`undo_history`] joins those entries to the audit ledger for a readable list.
//! - [`undo_enact`] turns a captured inverse back into a change on disk.

// `deny` rather than `forbid`: `rename_noreplace` needs one `libc::renameat2`
// call, which std does not expose and which is what makes a restore refuse to
// clobber a file that came back while the undo was pending. The one block is
// audited in place; nothing else here may add another.
#![deny(unsafe_code)]

pub mod undo_enact;
pub mod undo_history;
pub mod undo_signer;
