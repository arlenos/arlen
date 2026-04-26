pub mod annotations;
pub mod config;
pub mod event;
pub mod graph;
pub mod mock;
pub mod presence;
pub mod spatial;
pub mod timeline;

pub(crate) mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

pub use annotations::{
    AnnotationLookup, AnnotationRecord, AnnotationSetParams, AnnotationTarget, Annotations,
};
pub use event::{EmitError, EventEmitter, UnixEventEmitter};
pub use graph::{GraphClient, QueryError, UnixGraphClient};
pub use presence::{AutoClear, Presence, PresenceParams};
pub use spatial::{GeometryHint, OutputHint, Spatial, SpatialHint};
pub use timeline::{Timeline, TimelineParams};
pub mod shell_types;
