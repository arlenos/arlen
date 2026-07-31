//! Learning mode: the unmatched-app fallback (app-enrollment §E9). For an app
//! with no curated profile and no Flatpak `finish-args` (AppImage / tarball /
//! `~/.local/bin` script), the confiner runs complain-style (permit-and-log) and
//! this module derives the MINIMAL profile covering the observed accesses on top
//! of a permanent deny floor - the IAM-Access-Analyzer / generate-from-activity
//! pattern, never grant-all-seen.
//!
//! This is the pure derivation core: the deny floor, the path/host classification,
//! the minimal-profile synthesis, the trust tier, and the window/rate bounds. The
//! live confiner-observation hook that FEEDS the observed accesses is the
//! metal-coupled part, built separately.
//!
//! The four guardrails (non-negotiable):
//! 1. the deny floor - the Knowledge Graph is NEVER learnable; camera / mic /
//!    screen / location stay explicit and portal-gated; only filesystem and
//!    network are learnable;
//! 2. the output always carries the `learned/unverified` trust tier, visibly
//!    weaker than a curated profile;
//! 3. an enforcement breakage re-opens a short CONSENTED mini-window, never a
//!    silent auto-widen (this module bounds/rate-limits; the re-consent is the UI);
//! 4. the window is bounded (earliest of first-idle-after-init / a wall-clock cap
//!    / N launches) and `>5` new grants per minute suspends + raises an anomaly.

use std::path::{Path, PathBuf};

use crate::{FilesystemPermissions, NetworkPermissions, PermissionProfile, ProfileInfo};

/// Whether a permission dimension is learnable at all (guardrail 1). Only
/// filesystem and network are; the graph is never learnable, and
/// camera/mic/screen/location stay explicit and portal-gated.
pub fn is_learnable(dimension: &str) -> bool {
    matches!(dimension, "filesystem" | "network")
}

/// One access the confiner observed while the app ran under complain-mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// A file or directory the app read or wrote.
    Path(PathBuf),
    /// A network host the app connected to.
    Host(String),
}

/// The smallest filesystem scope an observed path generalises to: an XDG
/// dimension when the path is under a standard user directory, else the narrowest
/// enclosing directory under home (never `~/**`), else denied (outside home).
#[derive(Debug, Clone, PartialEq, Eq)]
enum FsScope {
    /// One of the six XDG dimensions (`home`/`documents`/...).
    Dimension(&'static str),
    /// A specific custom directory under home (never generalised to all of home).
    Custom(PathBuf),
    /// Outside the home directory: not learnable, dropped.
    Denied,
}

/// Classify an observed path to the smallest scope that covers it, relative to
/// the user's `home`. The XDG subdirs map to their dimension; a path elsewhere
/// under home generalises only to its immediate parent directory (never `~/**`,
/// never home itself); a path outside home is denied.
fn classify_path(path: &Path, home: &Path) -> FsScope {
    let Ok(rel) = path.strip_prefix(home) else {
        return FsScope::Denied;
    };
    let comps: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let Some(first) = comps.first() else {
        // The path IS home itself: granting it is granting all of `~/` - forbidden.
        return FsScope::Denied;
    };
    match *first {
        "Documents" => FsScope::Dimension("documents"),
        "Downloads" => FsScope::Dimension("downloads"),
        "Pictures" => FsScope::Dimension("pictures"),
        "Music" => FsScope::Dimension("music"),
        "Videos" => FsScope::Dimension("videos"),
        _ => {
            // A non-XDG path: grant only its immediate subdir under home, and
            // only when the access is INSIDE that subdir (>= 2 components). A
            // single item directly in home (a dotfile, a loose file) would
            // generalise to `~/` itself, which is forbidden - deny it.
            if comps.len() >= 2 {
                FsScope::Custom(home.join(first))
            } else {
                FsScope::Denied
            }
        }
    }
}

/// Derive the minimal permission profile covering `observed`, for `app_id`,
/// relative to the user's `home`. Only filesystem and network are learnable
/// (guardrail 1); filesystem accesses generalise to the smallest enclosing scope
/// and are deduped; any network host grants `network` (allow_all - the confiner
/// does not resolve per-host egress for a learned app). Never grant-all-seen: an
/// access outside home is dropped, not granted.
pub fn derive_minimal_profile(
    app_id: &str,
    home: &Path,
    observed: &[Observed],
) -> PermissionProfile {
    let mut fs = FilesystemPermissions::default();
    let mut custom = Vec::new();
    let mut network = false;

    for access in observed {
        match access {
            Observed::Host(_) => network = true,
            Observed::Path(p) => match classify_path(p, home) {
                FsScope::Dimension("documents") => fs.documents = true,
                FsScope::Dimension("downloads") => fs.downloads = true,
                FsScope::Dimension("pictures") => fs.pictures = true,
                FsScope::Dimension("music") => fs.music = true,
                FsScope::Dimension("videos") => fs.videos = true,
                FsScope::Dimension(_) => {}
                FsScope::Custom(dir) => {
                    if !custom.contains(&dir) {
                        custom.push(dir);
                    }
                }
                FsScope::Denied => {}
            },
        }
    }
    custom.sort();
    fs.custom = custom;

    PermissionProfile {
        info: ProfileInfo {
            app_id: app_id.to_string(),
            tier: crate::AppTier::ThirdParty,
        },
        graph: Default::default(),
        event_bus: Default::default(),
        filesystem: fs,
        network: NetworkPermissions {
            allow_all: network,
            ..Default::default()
        },
        notifications: Default::default(),
        clipboard: Default::default(),
        system: Default::default(),
        input: Default::default(),
        search: Default::default(),
        intents: Default::default(),
        mcp: Default::default(),
    }
}

/// The trust tier a learned profile carries: always `learned/unverified`,
/// visibly weaker than a curated profile (guardrail 2).
pub const LEARNED_TRUST_TIER: &str = "learned/unverified";

/// The learning window bounds (guardrail 4): learning ends at the EARLIEST of the
/// app first going idle after init, a wall-clock cap, or a launch-count cap.
#[derive(Debug, Clone, Copy)]
pub struct LearningWindow {
    /// Wall-clock cap in seconds (the default is ~10 minutes).
    pub wall_clock_secs: u64,
    /// Launch-count cap.
    pub max_launches: u32,
}

impl Default for LearningWindow {
    fn default() -> Self {
        Self {
            wall_clock_secs: 600,
            max_launches: 5,
        }
    }
}

impl LearningWindow {
    /// Whether the window has closed given the elapsed seconds, launch count, and
    /// whether the app has gone idle after init. Any one condition closes it.
    pub fn is_closed(&self, elapsed_secs: u64, launches: u32, idle_after_init: bool) -> bool {
        idle_after_init || elapsed_secs >= self.wall_clock_secs || launches >= self.max_launches
    }
}

/// The maximum new grants a learning app may acquire per minute before the window
/// is suspended and an anomaly is raised (guardrail 4).
pub const MAX_GRANTS_PER_MINUTE: u32 = 5;

/// Whether the observed new-grant rate exceeds the safe bound (`>5`/min), which
/// must suspend the learning window and raise an anomaly.
pub fn grant_rate_exceeded(new_grants_this_minute: u32) -> bool {
    new_grants_this_minute > MAX_GRANTS_PER_MINUTE
}

/// Where a learning session currently stands.
///
/// The two bounds above answer "has the window closed" and "is the rate too
/// high" as separate questions, and a caller that only asks them is missing the
/// thing guardrails 3 and 4 are actually about: a session that tripped the rate
/// cap must STAY stopped. Reading `grant_rate_exceeded` once, suspending, and
/// then asking again a minute later - when the per-minute count has naturally
/// fallen - would resume learning by itself, which is precisely the silent
/// auto-widen guardrail 3 forbids. Suspension is therefore a state, and only an
/// explicit re-consent leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearningState {
    /// Observing, and new grants may be accepted.
    Learning,
    /// The rate cap tripped. No further grants are accepted, and an anomaly is
    /// owed. Only [`LearningSession::reconsent`] resumes it.
    Suspended,
    /// The window ended normally. The profile derived so far is the result; no
    /// further grants are accepted, and this is terminal.
    Closed,
}

/// The learning window as a state machine over the bounds.
///
/// Deliberately not a timer or a task: it is fed observations and answers whether
/// a grant may be accepted, so the whole of guardrail 3 and 4 can be tested
/// against synthetic input, with the live clock and the confiner hook outside.
#[derive(Debug, Clone, Copy)]
pub struct LearningSession {
    window: LearningWindow,
    state: LearningState,
}

impl LearningSession {
    /// Open a session over `window`.
    pub fn new(window: LearningWindow) -> Self {
        Self {
            window,
            state: LearningState::Learning,
        }
    }

    /// The current state.
    pub fn state(&self) -> LearningState {
        self.state
    }

    /// Whether a newly observed access may still be folded into the profile.
    pub fn accepts_grants(&self) -> bool {
        self.state == LearningState::Learning
    }

    /// Feed the session the current bounds. Returns the state after the step.
    ///
    /// Ordering matters: the rate cap is checked FIRST, so a burst that arrives in
    /// the same step the window would have closed is still recorded as a
    /// suspension. Those are different outcomes - a closed window yields a profile,
    /// a suspended one yields a profile plus an anomaly and a demand for
    /// re-consent - and silently preferring the benign one would lose the signal
    /// that the app behaved abnormally.
    ///
    /// Neither `Suspended` nor `Closed` is left by observing more; both ignore
    /// further steps.
    pub fn observe(
        &mut self,
        elapsed_secs: u64,
        launches: u32,
        idle_after_init: bool,
        new_grants_this_minute: u32,
    ) -> LearningState {
        if self.state != LearningState::Learning {
            return self.state;
        }
        if grant_rate_exceeded(new_grants_this_minute) {
            self.state = LearningState::Suspended;
        } else if self.window.is_closed(elapsed_secs, launches, idle_after_init) {
            self.state = LearningState::Closed;
        }
        self.state
    }

    /// Resume a suspended session after the user explicitly re-consented
    /// (guardrail 3: a mini-window, never a silent auto-widen).
    ///
    /// A `Closed` session is NOT resumed: its window ended on its own terms and
    /// re-opening it would be a second learning window wearing the first one's
    /// name. Returns whether anything resumed.
    pub fn reconsent(&mut self) -> bool {
        if self.state == LearningState::Suspended {
            self.state = LearningState::Learning;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    #[test]
    fn a_suspended_session_does_not_resume_when_the_burst_passes() {
        // The bug the state exists to prevent. The rate cap trips, and a minute
        // later the per-minute count is naturally back to normal - a caller
        // re-asking `grant_rate_exceeded` would see false and carry on learning,
        // widening the profile with no one having consented to anything.
        let mut s = LearningSession::new(LearningWindow::default());
        assert_eq!(s.observe(10, 1, false, MAX_GRANTS_PER_MINUTE + 1), LearningState::Suspended);
        assert!(!s.accepts_grants());
        assert_eq!(s.observe(20, 1, false, 0), LearningState::Suspended);
        assert!(!s.accepts_grants(), "a quiet minute must not resume learning");
    }

    #[test]
    fn only_an_explicit_reconsent_resumes_a_suspended_session() {
        let mut s = LearningSession::new(LearningWindow::default());
        s.observe(10, 1, false, MAX_GRANTS_PER_MINUTE + 1);
        assert!(s.reconsent());
        assert!(s.accepts_grants());
        assert_eq!(s.state(), LearningState::Learning);
    }

    #[test]
    fn a_closed_window_is_terminal_and_reconsent_does_not_reopen_it() {
        // Re-opening a window that ended on its own terms would be a second
        // learning window wearing the first one's name.
        let mut s = LearningSession::new(LearningWindow::default());
        assert_eq!(s.observe(0, 0, true, 0), LearningState::Closed);
        assert!(!s.accepts_grants());
        assert!(!s.reconsent());
        assert_eq!(s.state(), LearningState::Closed);
    }

    #[test]
    fn a_burst_in_the_closing_step_is_a_suspension_not_a_clean_close() {
        // Both conditions in one step. Suspension wins because the outcomes
        // differ: a close yields a profile, a suspension yields a profile plus an
        // anomaly and a demand for re-consent, and preferring the benign reading
        // would lose the signal that the app behaved abnormally.
        let mut s = LearningSession::new(LearningWindow::default());
        let after = s.observe(9_999, 99, true, MAX_GRANTS_PER_MINUTE + 1);
        assert_eq!(after, LearningState::Suspended);
    }

    #[test]
    fn a_session_within_every_bound_keeps_learning() {
        let mut s = LearningSession::new(LearningWindow::default());
        assert_eq!(s.observe(1, 1, false, MAX_GRANTS_PER_MINUTE), LearningState::Learning);
        assert!(s.accepts_grants());
    }

    #[test]
    fn only_filesystem_and_network_are_learnable() {
        assert!(is_learnable("filesystem"));
        assert!(is_learnable("network"));
        assert!(!is_learnable("graph"));
        assert!(!is_learnable("camera"));
        assert!(!is_learnable("input"));
    }

    #[test]
    fn xdg_paths_generalise_to_their_dimension() {
        let obs = vec![
            Observed::Path(home().join("Documents/notes/todo.md")),
            Observed::Path(home().join("Downloads/a.zip")),
            Observed::Host("example.com".into()),
        ];
        let p = derive_minimal_profile("com.x", &home(), &obs);
        assert!(p.filesystem.documents);
        assert!(p.filesystem.downloads);
        assert!(!p.filesystem.pictures);
        assert!(p.network.allow_all);
        assert_eq!(p.info.tier, crate::AppTier::ThirdParty);
    }

    #[test]
    fn a_non_xdg_home_path_grants_only_its_immediate_subdir_never_home() {
        let obs = vec![Observed::Path(home().join("Projects/app/config.toml"))];
        let p = derive_minimal_profile("com.x", &home(), &obs);
        // Grants ~/Projects, never ~/ or a `**` glob, and no XDG dimension.
        assert!(!p.filesystem.home);
        assert_eq!(p.filesystem.custom, vec![home().join("Projects")]);
    }

    #[test]
    fn a_path_directly_in_home_or_outside_home_is_denied() {
        let obs = vec![
            Observed::Path(home().join(".bashrc")), // directly in home
            Observed::Path(PathBuf::from("/etc/passwd")), // outside home
        ];
        let p = derive_minimal_profile("com.x", &home(), &obs);
        assert!(!p.filesystem.home);
        assert!(p.filesystem.custom.is_empty());
    }

    #[test]
    fn the_window_closes_on_the_earliest_bound() {
        let w = LearningWindow::default();
        assert!(!w.is_closed(10, 1, false));
        assert!(w.is_closed(10, 1, true)); // idle
        assert!(w.is_closed(600, 1, false)); // wall clock
        assert!(w.is_closed(10, 5, false)); // launches
    }

    #[test]
    fn the_grant_rate_bound_is_five_per_minute() {
        assert!(!grant_rate_exceeded(5));
        assert!(grant_rate_exceeded(6));
    }
}
