//! The signer helper's error type.

/// An error in the undo-log signer helper.
#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    /// The HMAC key is missing, malformed, or unsafe to use, and no safe
    /// recovery is possible (a fresh key would invalidate an existing chain).
    #[error("undo-log signing key unavailable: {0}")]
    KeyUnavailable(String),
    /// A filesystem error resolving or preparing the signer's state directory.
    #[error("undo-log storage error: {0}")]
    Storage(String),
    /// An underlying I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The signer helper's result alias.
pub type Result<T> = std::result::Result<T, SignerError>;
