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

#[cfg(test)]
mod tests {
    use super::*;

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
}
