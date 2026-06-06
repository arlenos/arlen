//! The structured input to System Explanation Mode.
//!
//! Foundation §5.8 says the explanation correlates the **live event
//! stream** (which processes are active, what they read/write, which
//! network connections are open) with the **Knowledge Graph** (whether
//! this is normal, which project the files belong to, whether a process
//! has behaved unusually). [`SystemSnapshot`] is the assembled,
//! point-in-time view of both, ready to be rendered into the model
//! prompt.
//!
//! Everything here is machine-derived but still treated as **data, not
//! instructions**: a process name or file path is attacker-influenced
//! (a downloaded file, a renamed binary), so the prompt builder wraps
//! the whole snapshot in a content-origin-tagged `GRAPH-DATA` block
//! (S18-A) rather than interpolating it into the instruction channel.

/// One process currently active, as the event stream reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessActivity {
    /// The process or application name (e.g. `dnf`, `org.lunaris.files`).
    pub name: String,
    /// A short, human-readable note on what it is doing (e.g.
    /// `started ~2 min ago`, `indexing`), derived from recent events.
    pub detail: String,
}

/// One recent file access, correlated with the app and project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileActivity {
    /// The file path that was accessed.
    pub path: String,
    /// The application that accessed it.
    pub app: String,
    /// The project the file belongs to, if the graph knows one.
    pub project: Option<String>,
}

/// One open or recent network connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkActivity {
    /// The application that opened the connection.
    pub app: String,
    /// The destination host or address.
    pub destination: String,
    /// Whether the destination is within the application's declared
    /// permissions. `false` is surfaced as an anomaly in the summary.
    pub within_declared_permissions: bool,
}

/// The active project context, when one is in focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContext {
    /// The project name.
    pub name: String,
    /// How many files the graph associates with it.
    pub file_count: u64,
}

/// What kind of unusual situation an [`Anomaly`] flags. Foundation
/// §5.8 names the first two explicitly; the third covers the "is this
/// normal for this time of day on this machine" context the graph
/// provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyKind {
    /// A process accessed node types it has never accessed before.
    NovelNodeAccess,
    /// A network connection to a destination outside the application's
    /// declared permissions.
    UndeclaredNetworkDestination,
    /// Activity that is unusual for this time of day on this machine.
    UnusualForContext,
}

impl AnomalyKind {
    /// A stable, lowercase tag for rendering and tests.
    pub fn tag(self) -> &'static str {
        match self {
            AnomalyKind::NovelNodeAccess => "novel-node-access",
            AnomalyKind::UndeclaredNetworkDestination => "undeclared-network-destination",
            AnomalyKind::UnusualForContext => "unusual-for-context",
        }
    }
}

/// One flagged unusual situation. The summary mentions it in the same
/// natural-language response rather than logging it silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomaly {
    /// What kind of anomaly this is.
    pub kind: AnomalyKind,
    /// A short, human-readable description of the specific instance.
    pub description: String,
}

/// The assembled point-in-time view the explanation summarises: the
/// current moment (processes, files, network) plus graph context
/// (active project, anomalies). Built by the (future) graph/event
/// adapters and consumed by the prompt builder; nothing here performs
/// I/O.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemSnapshot {
    /// When the snapshot was taken, Unix seconds. Lets the model phrase
    /// the summary relative to "now" without a clock of its own.
    pub captured_at_unix: i64,
    /// Processes the event stream reports as currently active.
    pub processes: Vec<ProcessActivity>,
    /// Recent file accesses, correlated with app and project.
    pub files: Vec<FileActivity>,
    /// Open or recent network connections.
    pub network: Vec<NetworkActivity>,
    /// The active project, when one is in focus.
    pub active_project: Option<ProjectContext>,
    /// Anomalies the graph flagged against past behaviour and declared
    /// permissions.
    pub anomalies: Vec<Anomaly>,
}

impl SystemSnapshot {
    /// Whether the snapshot has no activity at all: a genuinely quiet
    /// system. The summary says so explicitly rather than inventing
    /// activity.
    pub fn is_quiet(&self) -> bool {
        self.processes.is_empty()
            && self.files.is_empty()
            && self.network.is_empty()
            && self.active_project.is_none()
            && self.anomalies.is_empty()
    }
}
