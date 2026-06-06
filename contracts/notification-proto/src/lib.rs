/// Shared protobuf definitions for the Lunaris notification system.
///
/// Generated from `proto/notification.proto` via prost. Used by both
/// the notification daemon and the desktop shell client.

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/lunaris.notification.rs"));
}

// Re-export commonly used types at the crate root.
pub use proto::*;
