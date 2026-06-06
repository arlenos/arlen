/// Socket server: shell-facing IPC for the module runtime daemon.
///
/// `protocol` defines the wire format (length-prefixed JSON, framed
/// `Request`/`Response`/`Event` envelopes). `server` accepts
/// connections, dispatches Requests to the manager, and broadcasts
/// Events to all listeners.

pub mod protocol;
pub mod server;

pub use protocol::{Event, Request, Response};
pub use server::SocketServer;
