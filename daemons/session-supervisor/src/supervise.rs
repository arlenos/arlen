//! Deciding what to do about one per-user unit, and why.
//!
//! The identity of a per-user daemon comes from the party that ASKED systemd to
//! start it, never from the unit's own name - `~/.config/systemd/user` outranks
//! `/usr/lib/systemd/user`, so a name is something the user can choose. This
//! supervisor is that party: it starts the enrolled units, registers the pid
//! systemd hands back, and registers again when systemd replaces it.
//!
//! # Why registering once is not enough
//!
//! Measured 13 Aug on real systemd: 16 of the 17 shipped per-user units carry
//! `Restart=on-failure`, and a restart gives a NEW MainPID within about a second
//! (3396268 -> 3396283 after a SIGKILL). A registration nobody renews therefore
//! names a dead process after the first crash, and by the standing rule nothing
//! may re-derive the identity from the unit's name afterwards - so there is no
//! recovery, and the daemon is simply unidentifiable from then on. It presents as
//! its peers being refused, which is indistinguishable from a policy decision.
//!
//! # The decision, and what is deliberately NOT here
//!
//! This module is the pure half: given what systemd reports and what we last
//! registered, what should happen. The D-Bus call and the pidfd live behind
//! [`Systemd`] and [`Registrar`] so the loop is tested against a scripted systemd
//! rather than a real one - the shape that let the restart case be tested at all.

use std::collections::BTreeMap;

/// What systemd reports about one unit. `MainPID` is 0 when the unit is not
/// running, which systemd uses for both "not started yet" and "between restarts",
/// so it is a distinct state rather than an error.
pub trait Systemd {
    /// Ask systemd to start `unit`. Idempotent: starting a running unit is a
    /// no-op, which matters because this runs again on every observed exit.
    fn start(&self, unit: &str) -> Result<(), String>;
    /// The unit's current MainPID, or 0 when nothing is running.
    fn main_pid(&self, unit: &str) -> Result<u32, String>;
}

/// The identity broker, as this supervisor uses it.
pub trait Registrar {
    /// Register `pid` as `app_id`. The pid is pinned by the caller (a pidfd) so
    /// the registration cannot drift onto a recycled pid.
    fn register(&self, pid: u32, app_id: &str) -> Result<(), String>;
}

/// What to do about a unit, this time round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing changed: the pid we registered is still the unit's MainPID.
    Unchanged,
    /// Register this pid - either the first one, or the replacement after a
    /// restart. Carries the previous pid when there was one, so the log can say
    /// which case it was rather than making a reader infer it.
    Register { pid: u32, replacing: Option<u32> },
    /// The unit reports no MainPID. Not an error and not a registration: systemd
    /// is between restarts, or has given up. Either way there is nothing to
    /// register and the next round will see the new pid if one arrives.
    NotRunning,
}

/// What this supervisor has registered for each unit it manages.
#[derive(Debug, Default)]
pub struct Registered(BTreeMap<String, u32>);

impl Registered {
    /// An empty ledger, as at session start.
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// The pid last registered for `unit`, if any.
    pub fn pid_for(&self, unit: &str) -> Option<u32> {
        self.0.get(unit).copied()
    }

    /// Record that `pid` is now the registered one for `unit`.
    pub fn record(&mut self, unit: &str, pid: u32) {
        self.0.insert(unit.to_string(), pid);
    }
}

/// Decide what one round should do about `unit`, given what systemd reports.
///
/// Pure, so the restart case is a two-line test rather than a kill and a sleep.
pub fn decide(observed_main_pid: u32, registered: Option<u32>) -> Action {
    if observed_main_pid == 0 {
        return Action::NotRunning;
    }
    match registered {
        Some(pid) if pid == observed_main_pid => Action::Unchanged,
        replacing => Action::Register {
            pid: observed_main_pid,
            replacing,
        },
    }
}

/// Run one round over every enrolled unit: start it if it is not running, and
/// register its MainPID if that is not the pid we already registered.
///
/// A failure on one unit does not abandon the others - a session where the
/// notification daemon is wedged should still have a working audit daemon - so
/// each unit's error is returned rather than propagated, and the caller logs it.
pub fn round(
    units: &[(&str, &str)],
    systemd: &dyn Systemd,
    registrar: &dyn Registrar,
    registered: &mut Registered,
) -> Vec<(String, Result<Action, String>)> {
    let mut out = Vec::new();
    for (unit, app_id) in units {
        out.push(((*unit).to_string(), one(unit, app_id, systemd, registrar, registered)));
    }
    out
}

/// One unit's round. Split out so the error path is one `?` chain.
fn one(
    unit: &str,
    app_id: &str,
    systemd: &dyn Systemd,
    registrar: &dyn Registrar,
    registered: &mut Registered,
) -> Result<Action, String> {
    // Unconditional and idempotent: on the first round this starts the unit, and
    // on a later one it is how a unit systemd gave up on (start-limit-hit) comes
    // back rather than staying down for the session's lifetime.
    systemd.start(unit)?;
    let action = decide(systemd.main_pid(unit)?, registered.pid_for(unit));
    if let Action::Register { pid, .. } = action {
        registrar.register(pid, app_id)?;
        registered.record(unit, pid);
    }
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_unit_that_is_not_running_is_neither_an_error_nor_a_registration() {
        assert_eq!(decide(0, None), Action::NotRunning);
        // Including between restarts, where we DO hold a stale pid: registering
        // nothing is right, and so is keeping the old entry - the next round sees
        // the replacement and reports it as replacing that pid.
        assert_eq!(decide(0, Some(1234)), Action::NotRunning);
    }

    #[test]
    fn the_first_pid_registers_and_the_same_pid_does_not_register_twice() {
        assert_eq!(decide(1234, None), Action::Register { pid: 1234, replacing: None });
        assert_eq!(decide(1234, Some(1234)), Action::Unchanged);
    }

    #[test]
    fn a_restart_registers_the_replacement_and_says_what_it_replaced() {
        // The measured case: SIGKILL, systemd restarts, MainPID moves. Without
        // this the registration names a dead process and the daemon is
        // unidentifiable for the rest of the session.
        assert_eq!(
            decide(3396283, Some(3396268)),
            Action::Register { pid: 3396283, replacing: Some(3396268) }
        );
    }

    /// A scripted systemd: each unit yields its MainPIDs in order, one per round.
    struct ScriptedSystemd {
        pids: RefCell<BTreeMap<String, Vec<u32>>>,
        starts: RefCell<Vec<String>>,
    }

    impl Systemd for ScriptedSystemd {
        fn start(&self, unit: &str) -> Result<(), String> {
            self.starts.borrow_mut().push(unit.to_string());
            Ok(())
        }
        fn main_pid(&self, unit: &str) -> Result<u32, String> {
            let mut pids = self.pids.borrow_mut();
            let queue = pids.get_mut(unit).ok_or_else(|| format!("no script for {unit}"))?;
            Ok(if queue.len() > 1 { queue.remove(0) } else { queue[0] })
        }
    }

    #[derive(Default)]
    struct RecordingRegistrar(RefCell<Vec<(u32, String)>>);

    impl Registrar for RecordingRegistrar {
        fn register(&self, pid: u32, app_id: &str) -> Result<(), String> {
            self.0.borrow_mut().push((pid, app_id.to_string()));
            Ok(())
        }
    }

    #[test]
    fn a_daemon_restarted_by_systemd_is_registered_again_under_the_same_id() {
        // Three rounds: it starts, it is stable, it restarts. Only the pid
        // changes; the app id never does, because the id belongs to the unit we
        // asked for and not to the process that happens to serve it.
        let systemd = ScriptedSystemd {
            pids: RefCell::new(BTreeMap::from([(
                "arlen-notifyd.service".to_string(),
                vec![100, 100, 200],
            )])),
            starts: RefCell::new(Vec::new()),
        };
        let registrar = RecordingRegistrar::default();
        let mut registered = Registered::new();
        let units = [("arlen-notifyd.service", "notifyd")];

        let first = round(&units, &systemd, &registrar, &mut registered);
        let second = round(&units, &systemd, &registrar, &mut registered);
        let third = round(&units, &systemd, &registrar, &mut registered);

        assert_eq!(first[0].1, Ok(Action::Register { pid: 100, replacing: None }));
        assert_eq!(second[0].1, Ok(Action::Unchanged));
        assert_eq!(third[0].1, Ok(Action::Register { pid: 200, replacing: Some(100) }));
        assert_eq!(
            *registrar.0.borrow(),
            vec![(100, "notifyd".to_string()), (200, "notifyd".to_string())],
            "the id is the unit's, not the process's"
        );
        // Started every round: idempotent for a running unit, and the way a unit
        // systemd gave up on comes back.
        assert_eq!(systemd.starts.borrow().len(), 3);
    }

    #[test]
    fn one_wedged_unit_does_not_abandon_the_others() {
        let systemd = ScriptedSystemd {
            pids: RefCell::new(BTreeMap::from([(
                "arlen-auditd.service".to_string(),
                vec![7],
            )])),
            starts: RefCell::new(Vec::new()),
        };
        let registrar = RecordingRegistrar::default();
        let mut registered = Registered::new();
        // The first unit has no script, so `main_pid` errors for it.
        let units = [
            ("arlen-notifyd.service", "notifyd"),
            ("arlen-auditd.service", "auditd"),
        ];
        let out = round(&units, &systemd, &registrar, &mut registered);
        assert!(out[0].1.is_err(), "the broken unit reports its error");
        assert_eq!(
            out[1].1,
            Ok(Action::Register { pid: 7, replacing: None }),
            "and the next unit is still handled"
        );
    }
}
