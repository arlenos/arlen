// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! One lock for the environment every test in this crate shares.
//!
//! `XDG_CONFIG_HOME` is per PROCESS, and three modules were pointing it at their
//! own temp dirs to isolate a config read: `quicksettings::layout` behind a
//! module-local `TEST_ENV_LOCK`, `clipboard_history` behind a module-local
//! `ENV_LOCK`, and `settings_provider` behind nothing at all - which also never
//! put the variable back, so it leaked into every test that ran after it on that
//! thread.
//!
//! Two locks over one variable is no lock. `with_isolated_config` promised
//! isolation it could not give, and the symptom was
//! `move_tile_shifts_position` failing under a parallel `cargo test` and passing
//! under `--test-threads=1`: a red that is not a defect, which is the kind that
//! teaches people to stop reading reds.
//!
//! So there is one lock here, and every test that touches the process
//! environment takes it.

use std::sync::{Mutex, MutexGuard};

static ENV: Mutex<()> = Mutex::new(());

/// Hold the environment for the duration of a test.
///
/// Poison is recovered rather than propagated: a test that panicked while
/// holding this left the variable in whatever state it had, and failing every
/// LATER test with a poison error hides the one that actually broke.
pub fn lock() -> MutexGuard<'static, ()> {
    ENV.lock().unwrap_or_else(|p| p.into_inner())
}

/// Run `f` with `XDG_CONFIG_HOME` pointed at a fresh temp dir, then put the
/// previous value back.
///
/// Restores rather than removes, because a developer running the suite has a
/// real `XDG_CONFIG_HOME` and a test that deletes it changes what every later
/// test reads.
pub fn with_config_home<R>(f: impl FnOnce(&std::path::Path) -> R) -> R {
    let _g = lock();
    let tmp = tempfile::tempdir().expect("temp dir");
    let prev = std::env::var_os("XDG_CONFIG_HOME");
    // SAFETY: the lock above makes this the only thread touching the
    // environment for as long as `f` runs.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };
    let out = f(tmp.path());
    unsafe {
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
    out
}
