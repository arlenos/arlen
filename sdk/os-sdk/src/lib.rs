pub mod ambient;
pub mod annotations;
pub mod badges;
pub mod clipboard;
pub mod config;
pub mod event;
pub mod event_consumer;
pub mod graph;
pub mod mock;
pub mod intents;
pub mod mcp;
pub mod presence;
pub mod search;
pub mod shortcuts;
pub mod spatial;
pub mod timeline;
pub mod toolbar;

/// The Event Bus wire types (the `lunaris.eventbus` protobuf package).
///
/// Public so an event *consumer* can decode the type-specific payload of a
/// subscribed [`Event`] (the `subscribe()` channel yields envelopes; the
/// payload bytes deserialize into the matching message here, e.g.
/// `FileOpenedPayload`). This is the wire contract the bus and its
/// producers already share.
pub mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

pub(crate) mod proto_clipboard {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.clipboard.rs"));
}

pub(crate) mod proto_search {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.search.rs"));
}

pub(crate) mod proto_intents {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.intents.rs"));
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
pub use proto::Event;
pub use graph::{GraphClient, QueryError, UnixGraphClient};
pub use intents::{IntentError, IntentType, UnixIntentClient};
pub use presence::{AutoClear, Presence, PresenceParams};
pub use search::{SearchError, SearchMode, UnixSearchClient};
pub use spatial::{GeometryHint, OutputHint, Spatial, SpatialHint};
pub use timeline::{Timeline, TimelineParams};
pub use toolbar::{
    decode_action_invoked, ActionInvoked, BreadcrumbItem, ProgressState, QuickAction,
    Toolbar, MAX_QUICK_ACTIONS,
};
pub use ambient::{
    Ambient, AmbientColor, AmbientEffect, AmbientParams, AmbientSpeed, MAX_INTENSITY,
};
pub use badges::{BadgeKind, BadgeStatus, Badges};
pub use shortcuts::{
    decode_shortcut_invoked, Shortcut, ShortcutInvoked, ShortcutState, Shortcuts,
};
pub mod shell_types;
