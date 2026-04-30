pub mod annotations;
pub mod clipboard;
pub mod config;
pub mod event;
pub mod event_consumer;
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

pub(crate) mod proto_clipboard {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.clipboard.rs"));
}

pub use annotations::{
    AbortOnDrop, AnnotationChange, AnnotationLookup, AnnotationRecord,
    AnnotationSetParams, AnnotationTarget, Annotations, Subscription,
};
pub use clipboard::{
    ClipboardEntry, ClipboardError, ClipboardLabel, MockClipboardClient, UnixClipboardClient,
    WriteParams,
};
pub use event::{EmitError, EventEmitter, UnixEventEmitter};
pub use event_consumer::{EventConsumer, SubscribeError, UnixEventConsumer};
pub use graph::{GraphClient, QueryError, UnixGraphClient};
pub use presence::{AutoClear, Presence, PresenceParams};
pub use spatial::{GeometryHint, OutputHint, Spatial, SpatialHint};
pub use timeline::{Timeline, TimelineParams};
pub mod shell_types;
