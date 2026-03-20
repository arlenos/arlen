pub mod config;
pub mod event;
pub mod graph;
pub mod mock;

pub(crate) mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

pub use event::{EmitError, EventEmitter, UnixEventEmitter};
pub use graph::{GraphClient, QueryError, UnixGraphClient};
