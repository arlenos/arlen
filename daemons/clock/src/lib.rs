//! The clock daemon's state and the arithmetic behind it.
//!
//! **The GUI owns nothing.** `clock-app.md` §1: the Tauri app is a view, it may
//! be closed at any time, and closing it must change nothing. The daemon holds
//! the alarms, arms the kernel timers, rings, and feeds the topbar. That is not
//! a layering preference - an alarm that stops existing when a window closes is
//! not an alarm.
//!
//! The consequence for this crate's shape: the state it serves carries **anchor
//! timestamps** - when an alarm next rings, when a timer ends, when a run
//! started - and never counters. A counter served over IPC is a number that was
//! true when it was sent; an anchor stays true, and the view derives the
//! countdown from it. The frontend is already built to that contract.

pub mod alarm;
pub mod due;
pub mod focus;
pub mod missed;
pub mod reduce;
pub mod ring;
pub mod run;
pub mod startup;
pub mod state;
pub mod store;
pub mod wake;
