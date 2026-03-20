use std::future::Future;

/// Emits structured events onto the Event Bus.
/// Implemented by the real Unix socket client and a mock for tests.
pub trait EventEmitter: Send + Sync {
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a;
}

#[derive(Debug)]
pub enum EmitError {
    ConnectionFailed(String),
    SerializationFailed(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
            EmitError::SerializationFailed(msg) => write!(f, "serialization failed: {msg}"),
        }
    }
}

impl std::error::Error for EmitError {}
