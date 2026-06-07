//! System Explanation Mode for the Arlen AI layer.
//!
//! Implements Foundation §5.8: answers "What is my computer doing right
//! now?" by correlating the live event stream with the Knowledge Graph
//! and producing a dynamically generated plain-language summary via the
//! configured AI provider. It is a query like any other, executed only
//! when the user asks; nothing runs in the background and nothing is
//! reported automatically. Everything it uses is local.
//!
//! This crate is built in increments behind seams, like the rest of the
//! AI layer:
//!
//! 1. **Snapshot contract + prompt builder (this increment).**
//!    [`snapshot::SystemSnapshot`] is the assembled point-in-time view
//!    (processes, files, network, active project, anomalies);
//!    [`prompt::build_explanation_prompt`] turns it into a single
//!    content-origin-tagged prompt (S18-A `GRAPH-DATA`), pure and
//!    model-free.
//! 2. **Graph context source (this increment).**
//!    [`source::graph_context`] fills the graph-derivable half of a
//!    `SystemSnapshot` (recent files + active project) behind a
//!    read-only [`source::GraphReader`] seam, with
//!    [`source::UnixGraphReader`] over the os-sdk graph client. The
//!    live-moment fields (processes, network) and anomalies have their
//!    own sources, folded in by the caller.
//! 3. **Orchestration (this increment).** [`explain`] turns an
//!    assembled snapshot into a plain-language summary via the
//!    `AIProvider` seam; [`explain_system`] wires the graph-context
//!    source for today's callers.
//! 4. **Daemon wiring (next).** Expose it as the ai-daemon
//!    `org.arlen.AI1` `explain_system()` D-Bus method, assembling a
//!    full snapshot once the live-moment and anomaly sources exist.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod explain;
pub mod prompt;
pub mod snapshot;
pub mod source;

pub use explain::{explain, explain_system, explain_with_sources, ExplainError};
pub use prompt::{build_explanation_prompt, render_snapshot};
pub use snapshot::{
    Anomaly, AnomalyKind, Coverage, FileActivity, NetworkActivity, ProcessActivity,
    ProjectContext, SystemSnapshot,
};
pub use source::{
    anomaly_context, graph_context, live_context, merge_snapshots, AnomalyReader,
    FileAnomalyReader, GraphReader, ProcProcessReader, ProcessReader, SnapshotError,
    UnixGraphReader, REQUIRED_GRAPH_LABELS,
};
