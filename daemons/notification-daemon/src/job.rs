//! The unified job/progress model (job-progress-surface.md), the KDE
//! JobViewV3-derived contract the shell's Activity/Jobs zone renders.
//!
//! This is the pure data model plus the load-bearing progress logic; the D-Bus
//! JobView object and the producers (file-manager, forage, model-manager) build
//! on it. The design choices that live here as testable code:
//!
//!  - amounts are carried in REAL units (bytes/files/items), never a pre-baked
//!    percent - the consumer renders "13 of 19 files" and derives the bar;
//!  - the 0..1 bar fraction is MONOTONIC (it never goes backwards, even if a
//!    later total shrinks or the processed count resets), kept SEPARATE from the
//!    non-monotonic ETA/speed a producer reports elsewhere;
//!  - a job may FLIP from indeterminate to determinate mid-run (count first,
//!    then show the bar).

/// The real unit a job's amounts are counted in. The consumer renders the
/// human string ("12 of 30 MB", "13 of 19 files"); the model just carries the
/// unit so the renderer can format it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Byte counts (a download, a copy).
    Bytes,
    /// File counts (a multi-file move).
    Files,
    /// Directory counts.
    Directories,
    /// A generic item count (photos, mails, conversions).
    Items,
}

impl Unit {
    /// The stable wire token (a producer names its unit as a string over D-Bus).
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Bytes => "bytes",
            Unit::Files => "files",
            Unit::Directories => "directories",
            Unit::Items => "items",
        }
    }

    /// Parse a wire token. An unknown token maps to the generic `Items` (never a
    /// failure: a producer's odd unit still counts, it just renders generically).
    pub fn from_wire(token: &str) -> Unit {
        match token {
            "bytes" => Unit::Bytes,
            "files" => Unit::Files,
            "directories" => Unit::Directories,
            _ => Unit::Items,
        }
    }
}

/// A job's lifecycle state (mirrors KDE's JobViewV3). Every non-running state
/// carries an explanatory message on the [`JobView`] so the shell can say WHY a
/// job is not progressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Actively progressing.
    Running,
    /// Suspended by the user; resumable (the producer declared `suspendable`).
    Paused,
    /// Blocked on something outside the producer (a full disk, a lost mount) -
    /// not an error, will resume itself when the impediment clears.
    Impeded,
    /// Failed but retryable (a transient network drop); the shell may offer retry.
    ErrorRecoverable,
    /// Failed terminally; the job is over.
    ErrorFatal,
    /// Completed successfully.
    Done,
}

impl JobState {
    /// Whether the job has reached an end state (no further updates expected).
    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Done | JobState::ErrorFatal)
    }

    /// Whether the shell should render an explanatory message: every state but
    /// `Running` explains itself.
    pub fn needs_message(self) -> bool {
        !matches!(self, JobState::Running)
    }

    /// The stable wire token for D-Bus.
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Impeded => "impeded",
            JobState::ErrorRecoverable => "error-recoverable",
            JobState::ErrorFatal => "error-fatal",
            JobState::Done => "done",
        }
    }

    /// Parse a wire token. An unknown token yields `None` (fail-closed: an
    /// unrecognised state is rejected rather than silently coerced, so a
    /// producer typo cannot mislabel a job's lifecycle).
    pub fn from_wire(token: &str) -> Option<JobState> {
        match token {
            "running" => Some(JobState::Running),
            "paused" => Some(JobState::Paused),
            "impeded" => Some(JobState::Impeded),
            "error-recoverable" => Some(JobState::ErrorRecoverable),
            "error-fatal" => Some(JobState::ErrorFatal),
            "done" => Some(JobState::Done),
            _ => None,
        }
    }
}

/// What the producer permits the shell to request back (à la KJob capabilities).
/// The shell only draws a Cancel/Pause affordance for a capability the producer
/// declared, so a request always has a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobCapabilities {
    /// The job can be cancelled/killed (Cancel = clean, Stop = keep partial).
    pub killable: bool,
    /// The job can be suspended and resumed (a pause that can actually resume;
    /// never offered for a job that cannot).
    pub suspendable: bool,
}

/// A job's progress: the raw amounts in real units plus the derived monotonic
/// 0..1 bar fraction.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    unit: Unit,
    processed: u64,
    /// `None` while indeterminate (no known total); set once a total is known,
    /// which flips the job to determinate.
    total: Option<u64>,
    /// The monotonic bar fraction in `0.0..=1.0`. Never lowered by an update.
    fraction: f64,
}

impl Progress {
    /// A fresh indeterminate progress (no total yet): the bar shows activity but
    /// no fraction until a total arrives.
    pub fn indeterminate(unit: Unit) -> Self {
        Progress {
            unit,
            processed: 0,
            total: None,
            fraction: 0.0,
        }
    }

    /// A fresh determinate progress with a known total.
    pub fn determinate(unit: Unit, total: u64) -> Self {
        Progress {
            unit,
            processed: 0,
            total: Some(total),
            fraction: 0.0,
        }
    }

    /// Record new amounts. A `Some(total)` sets/updates the total (and flips an
    /// indeterminate job to determinate); `None` leaves the current total. The
    /// bar fraction is recomputed and advanced but NEVER lowered - a later total
    /// that shrinks, or a processed count that resets, holds the bar rather than
    /// yanking it backwards (KDE's bar-never-regresses rule). A zero or absent
    /// total leaves the fraction untouched (indeterminate stretch).
    pub fn update(&mut self, processed: u64, total: Option<u64>) {
        self.processed = processed;
        if total.is_some() {
            self.total = total;
        }
        let raw = match self.total {
            Some(t) if t > 0 => (processed as f64 / t as f64).clamp(0.0, 1.0),
            _ => self.fraction,
        };
        if raw > self.fraction {
            self.fraction = raw;
        }
    }

    /// The monotonic bar fraction, `0.0..=1.0`.
    pub fn fraction(&self) -> f64 {
        self.fraction
    }

    /// Whether a total is known (the bar can be drawn), vs indeterminate.
    pub fn is_determinate(&self) -> bool {
        self.total.is_some()
    }

    /// The processed amount in [`Unit`].
    pub fn processed(&self) -> u64 {
        self.processed
    }

    /// The total amount, when known.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// The unit the amounts are counted in.
    pub fn unit(&self) -> Unit {
        self.unit
    }
}

/// One long-running operation the shell aggregates. The stable `id` + `app_id`
/// identify it; `title` is the human line; the rest is the render + control
/// contract. The `egress_host` is the one sovereign field: a job that reaches
/// the network names where, so the shell can surface it at the consent moment.
#[derive(Debug, Clone, PartialEq)]
pub struct JobView {
    /// Stable id for the job's lifetime (assigned by the server on register).
    pub id: u64,
    /// The producing app's attested identity.
    pub app_id: String,
    /// The human title ("Copying 240 photos to USB").
    pub title: String,
    /// Progress in real units + the monotonic bar.
    pub progress: Progress,
    /// The lifecycle state.
    pub state: JobState,
    /// Why the job is in a non-running state (a full disk, a network drop). Set
    /// exactly when `state.needs_message()`.
    pub state_message: Option<String>,
    /// What the shell may request back.
    pub capabilities: JobCapabilities,
    /// Start time, epoch micros (for the elapsed line + ETA).
    pub started_at: u64,
    /// The sovereign field: the host this job reaches, when it egresses.
    pub egress_host: Option<String>,
}

/// The fields a producer supplies to register a new job. The registry assigns
/// the stable `id` and starts it in [`JobState::Running`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    /// The producing app's attested identity.
    pub app_id: String,
    /// The human title.
    pub title: String,
    /// The initial progress (determinate or indeterminate).
    pub progress: Progress,
    /// What the shell may request back.
    pub capabilities: JobCapabilities,
    /// The host this job reaches, when it egresses.
    pub egress_host: Option<String>,
    /// Start time, epoch micros.
    pub started_at: u64,
}

/// The in-memory store of active jobs (job-progress-surface.md). The D-Bus
/// JobView object and the shell client operate over this: a producer registers,
/// updates, and finishes a job; the shell reads a [`snapshot`](JobRegistry::snapshot).
/// Ids are stable for the registry's lifetime and never reused, so a client can
/// never confuse a fresh job for a finished one that shared a slot.
#[derive(Debug, Default)]
pub struct JobRegistry {
    jobs: std::collections::BTreeMap<u64, JobView>,
    next_id: u64,
}

impl JobRegistry {
    /// A fresh, empty registry. The first registered job gets id 1.
    pub fn new() -> Self {
        JobRegistry {
            jobs: std::collections::BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Register a new job and return its stable id. Starts `Running` with no
    /// message.
    pub fn register(&mut self, spec: NewJob) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.insert(
            id,
            JobView {
                id,
                app_id: spec.app_id,
                title: spec.title,
                progress: spec.progress,
                state: JobState::Running,
                state_message: None,
                capabilities: spec.capabilities,
                started_at: spec.started_at,
                egress_host: spec.egress_host,
            },
        );
        id
    }

    /// The job with `id`, if it is still registered.
    pub fn get(&self, id: u64) -> Option<&JobView> {
        self.jobs.get(&id)
    }

    /// Advance a job's progress amounts. Returns `false` for an unknown id (a
    /// late update after removal is a no-op, never a panic).
    pub fn update_progress(&mut self, id: u64, processed: u64, total: Option<u64>) -> bool {
        match self.jobs.get_mut(&id) {
            Some(job) => {
                job.progress.update(processed, total);
                true
            }
            None => false,
        }
    }

    /// Set a job's state and its explanatory message (supply one for every
    /// non-running state; `None` for `Running`). Returns `false` for an unknown
    /// id. A terminal state does NOT auto-remove the job - the server prunes it
    /// after the shell's min-dwell so a `Done` flash is still seen.
    pub fn set_state(&mut self, id: u64, state: JobState, message: Option<String>) -> bool {
        match self.jobs.get_mut(&id) {
            Some(job) => {
                job.state = state;
                job.state_message = message;
                true
            }
            None => false,
        }
    }

    /// Remove a finished job. Returns whether it existed. The id is not reused.
    pub fn remove(&mut self, id: u64) -> bool {
        self.jobs.remove(&id).is_some()
    }

    /// A snapshot of every active job, ordered by id (stable for the shell's
    /// list rendering). Cloned so the caller holds no lock on the registry.
    pub fn snapshot(&self) -> Vec<JobView> {
        self.jobs.values().cloned().collect()
    }

    /// The number of active jobs.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Whether no job is active.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

/// The D-Bus-facing job server logic: the wire-value adapter over the shared
/// [`JobRegistry`]. A producer names its unit and state as strings; this parses
/// them (a bad unit falls back to `Items`, a bad state is rejected), builds the
/// model, and delegates to the registry under the lock. The zbus `#[interface]`
/// (a thin wrapper that owns the well-known name allow-replace) and the
/// shell-notification broadcast are the plumbing layers above this.
#[derive(Clone)]
pub struct JobViewServer {
    registry: std::sync::Arc<std::sync::Mutex<JobRegistry>>,
}

impl JobViewServer {
    /// Build over a shared registry.
    pub fn new(registry: std::sync::Arc<std::sync::Mutex<JobRegistry>>) -> Self {
        JobViewServer { registry }
    }

    /// Register a job from wire values; `total = None` starts it indeterminate.
    /// Returns the stable id.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &self,
        app_id: String,
        title: String,
        unit: &str,
        total: Option<u64>,
        killable: bool,
        suspendable: bool,
        egress_host: Option<String>,
        started_at: u64,
    ) -> u64 {
        let unit = Unit::from_wire(unit);
        let progress = match total {
            Some(t) => Progress::determinate(unit, t),
            None => Progress::indeterminate(unit),
        };
        let spec = NewJob {
            app_id,
            title,
            progress,
            capabilities: JobCapabilities {
                killable,
                suspendable,
            },
            egress_host,
            started_at,
        };
        self.lock().register(spec)
    }

    /// Advance a job's amounts (unknown id -> false).
    pub fn update(&self, id: u64, processed: u64, total: Option<u64>) -> bool {
        self.lock().update_progress(id, processed, total)
    }

    /// Set a job's state from a wire token + message. An unknown token or id ->
    /// false (the caller reports the rejection; the job is untouched).
    pub fn set_state(&self, id: u64, state: &str, message: Option<String>) -> bool {
        match JobState::from_wire(state) {
            Some(s) => self.lock().set_state(id, s, message),
            None => false,
        }
    }

    /// Remove a finished job (after the shell's dwell). Returns whether it existed.
    pub fn remove(&self, id: u64) -> bool {
        self.lock().remove(id)
    }

    /// A snapshot of every active job for the shell.
    pub fn snapshot(&self) -> Vec<JobView> {
        self.lock().snapshot()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, JobRegistry> {
        self.registry.lock().expect("job registry mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(app: &str, title: &str) -> NewJob {
        NewJob {
            app_id: app.to_string(),
            title: title.to_string(),
            progress: Progress::determinate(Unit::Files, 10),
            capabilities: JobCapabilities {
                killable: true,
                suspendable: false,
            },
            egress_host: None,
            started_at: 1,
        }
    }

    #[test]
    fn register_assigns_stable_increasing_ids() {
        let mut r = JobRegistry::new();
        let a = r.register(spec("files", "Copy A"));
        let b = r.register(spec("files", "Copy B"));
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(r.get(a).unwrap().title, "Copy A");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn update_and_set_state_mutate_the_job_and_reject_unknown_ids() {
        let mut r = JobRegistry::new();
        let id = r.register(spec("files", "Copy"));
        assert!(r.update_progress(id, 5, None));
        assert!((r.get(id).unwrap().progress.fraction() - 0.5).abs() < 1e-9);
        assert!(r.set_state(id, JobState::Impeded, Some("disk full".into())));
        assert_eq!(r.get(id).unwrap().state, JobState::Impeded);
        assert_eq!(r.get(id).unwrap().state_message.as_deref(), Some("disk full"));
        // Unknown id: a no-op, not a panic.
        assert!(!r.update_progress(999, 1, None));
        assert!(!r.set_state(999, JobState::Done, None));
    }

    #[test]
    fn remove_drops_the_job_and_the_id_is_never_reused() {
        let mut r = JobRegistry::new();
        let a = r.register(spec("files", "A"));
        assert!(r.remove(a));
        assert!(!r.remove(a), "a second remove is a no-op");
        assert!(r.get(a).is_none());
        // The next registration does NOT reuse the freed id.
        let b = r.register(spec("files", "B"));
        assert_eq!(b, 2, "ids are never reused");
        assert!(r.is_empty() || r.len() == 1);
    }

    #[test]
    fn snapshot_lists_active_jobs_ordered_by_id() {
        let mut r = JobRegistry::new();
        r.register(spec("files", "A"));
        r.register(spec("forage", "B"));
        r.register(spec("model", "C"));
        let snap = r.snapshot();
        let titles: Vec<&str> = snap.iter().map(|j| j.title.as_str()).collect();
        assert_eq!(titles, ["A", "B", "C"], "ordered by ascending id");
    }

    #[test]
    fn the_bar_fraction_advances_with_processed() {
        let mut p = Progress::determinate(Unit::Files, 20);
        assert_eq!(p.fraction(), 0.0);
        p.update(5, None);
        assert!((p.fraction() - 0.25).abs() < 1e-9);
        p.update(10, None);
        assert!((p.fraction() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn the_bar_never_regresses_when_processed_resets() {
        let mut p = Progress::determinate(Unit::Bytes, 100);
        p.update(80, None);
        assert!((p.fraction() - 0.8).abs() < 1e-9);
        // The producer re-reports a lower processed count: the bar holds at 0.8.
        p.update(30, None);
        assert!((p.fraction() - 0.8).abs() < 1e-9, "the bar does not go backwards");
    }

    #[test]
    fn the_bar_never_regresses_when_the_total_grows() {
        let mut p = Progress::determinate(Unit::Files, 10);
        p.update(9, None);
        assert!((p.fraction() - 0.9).abs() < 1e-9);
        // The scan discovers more files (total 10 -> 100): the raw fraction would
        // drop to 0.09, but the bar holds at 0.9.
        p.update(9, Some(100));
        assert!((p.fraction() - 0.9).abs() < 1e-9, "a grown total never yanks the bar back");
        assert_eq!(p.total(), Some(100), "the new total is recorded for the count line");
    }

    #[test]
    fn a_job_flips_from_indeterminate_to_determinate_mid_run() {
        let mut p = Progress::indeterminate(Unit::Items);
        assert!(!p.is_determinate());
        assert_eq!(p.fraction(), 0.0);
        // Counting done: a total arrives, the bar can now be drawn.
        p.update(3, Some(12));
        assert!(p.is_determinate());
        assert!((p.fraction() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_zero_total_stays_indeterminate_without_dividing_by_zero() {
        let mut p = Progress::determinate(Unit::Bytes, 0);
        p.update(5, None);
        // No division by zero; the fraction is untouched.
        assert_eq!(p.fraction(), 0.0);
    }

    #[test]
    fn the_fraction_is_clamped_to_one() {
        let mut p = Progress::determinate(Unit::Files, 10);
        // A producer over-reports processed > total: the bar caps at full.
        p.update(15, None);
        assert!((p.fraction() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn state_terminality_and_message_need() {
        assert!(JobState::Done.is_terminal());
        assert!(JobState::ErrorFatal.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::Paused.is_terminal());
        assert!(!JobState::Running.needs_message());
        assert!(JobState::Impeded.needs_message());
        assert!(JobState::ErrorRecoverable.needs_message());
    }

    #[test]
    fn wire_tokens_round_trip() {
        for s in [
            JobState::Running,
            JobState::Paused,
            JobState::Impeded,
            JobState::ErrorRecoverable,
            JobState::ErrorFatal,
            JobState::Done,
        ] {
            assert_eq!(JobState::from_wire(s.as_str()), Some(s));
        }
        assert_eq!(JobState::from_wire("bogus"), None, "an unknown state is rejected");
        for u in [Unit::Bytes, Unit::Files, Unit::Directories, Unit::Items] {
            assert_eq!(Unit::from_wire(u.as_str()), u);
        }
        assert_eq!(
            Unit::from_wire("widgets"),
            Unit::Items,
            "an unknown unit falls back to Items"
        );
    }

    fn server() -> JobViewServer {
        JobViewServer::new(std::sync::Arc::new(std::sync::Mutex::new(JobRegistry::new())))
    }

    #[test]
    fn server_registers_from_wire_values() {
        let s = server();
        let id = s.register("files".into(), "Copy".into(), "bytes", Some(1000), true, false, None, 1);
        let snap = s.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].id, id);
        assert_eq!(snap[0].progress.unit(), Unit::Bytes);
        assert!(snap[0].progress.is_determinate());
        assert!(snap[0].capabilities.killable);
    }

    #[test]
    fn server_handles_indeterminate_bad_unit_and_bad_state() {
        let s = server();
        let id = s.register("app".into(), "Scan".into(), "widgets", None, false, false, None, 1);
        {
            let j = s.snapshot();
            assert!(!j[0].progress.is_determinate(), "no total -> indeterminate");
            assert_eq!(j[0].progress.unit(), Unit::Items, "unknown unit -> Items");
        }
        assert!(s.update(id, 3, Some(6)));
        assert!(s.set_state(id, "impeded", Some("waiting".into())));
        assert!(!s.set_state(id, "nonsense", None), "a bad state token is rejected");
        assert_eq!(s.snapshot()[0].state, JobState::Impeded);
        // update/state on an unknown id is a no-op.
        assert!(!s.update(999, 1, None));
        assert!(s.remove(id));
    }
}
