//! System Explanation Mode for the Lunaris AI layer.
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
//! 2. **Graph/event adapters (next).** Fill a `SystemSnapshot` from the
//!    Knowledge Graph (recent activity, project context, anomaly
//!    signals) behind a seam, with a real implementation over the
//!    os-sdk graph client.
//! 3. **Orchestration + daemon wiring (after).** `explain_system()`
//!    over the `AIProvider` seam, exposed as the ai-daemon
//!    `org.lunaris.AI1` D-Bus method.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod prompt;
pub mod snapshot;

pub use prompt::{build_explanation_prompt, render_snapshot};
pub use snapshot::{
    Anomaly, AnomalyKind, FileActivity, NetworkActivity, ProcessActivity, ProjectContext,
    SystemSnapshot,
};
