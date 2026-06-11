//! The confinement boundary for reading untrusted third-party config.
//!
//! A config file an app or the agent did not write is untrusted input on two
//! axes, and this module closes both before a [`FormatHandler`](crate::FormatHandler)
//! ever parses it:
//!
//!  - **Filesystem reach.** The read goes through a cap-std [`Dir`] capability
//!    rooted at a directory the caller already holds. `Dir::open` resolves the
//!    relative path within that root and refuses a `..` traversal or an absolute
//!    path, so a malicious path cannot escape the capability to read an
//!    arbitrary host file.
//!  - **Parser exposure.** The raw bytes are stripped to inert plain text in the
//!    S18-B parse sandbox ([`arlen_ai_sandbox::parse_document`]) before a handler
//!    parses them. The sandbox runs the extraction in a separate locked-down
//!    subprocess (no network, no filesystem) and returns only inert text (ANSI
//!    escapes, control characters and invisible/bidi format characters stripped),
//!    so a crafted document cannot smuggle hidden content into the model and a
//!    parser exploited by it cannot reach the network or the graph.
//!
//! The sandbox is reused, not reinvented: the caller passes the path to the
//! `arlen-doc-sandbox` worker binary. A caller that already holds trusted,
//! in-process bytes parses them directly through a handler and does not need this
//! module.

use std::io::Read;
use std::path::Path;

use cap_std::fs::Dir;

use crate::error::ParseError;
use crate::model::ConfigModel;
use crate::{handler_for, Format, MAX_CONFIG_BYTES};

/// A failure reading or parsing a confined config file. Every variant means no
/// trustworthy model was produced; the caller treats it as fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum ConfinedError {
    /// The file could not be opened within the capability, or the read failed.
    /// A path that tries to escape the capability root (a `..` traversal or an
    /// absolute path) fails here.
    #[error("confined read failed: {0}")]
    Read(String),

    /// The file exceeded [`MAX_CONFIG_BYTES`]. Refused, not truncated and parsed.
    #[error("config too large")]
    TooLarge,

    /// The S18-B parse sandbox refused or failed to strip the bytes to inert
    /// text. No text crossed the boundary, so nothing is parsed.
    #[error("sandbox failed: {0}")]
    Sandbox(String),

    /// The inert text the sandbox produced did not parse as the chosen format.
    #[error(transparent)]
    Parse(#[from] ParseError),
}

/// Read a config file's bytes through the cap-std `dir` capability, bounded by
/// [`MAX_CONFIG_BYTES`].
///
/// `rel` is resolved within `dir`; a `..` traversal or an absolute path is
/// rejected by cap-std rather than followed, so the read cannot escape the
/// capability root. The byte cap is enforced by reading one byte past it and
/// refusing an oversize file, so a hostile multi-megabyte file is refused, not
/// walked.
pub fn read_confined(dir: &Dir, rel: impl AsRef<Path>) -> Result<Vec<u8>, ConfinedError> {
    let file = dir
        .open(rel.as_ref())
        .map_err(|e| ConfinedError::Read(format!("{e}")))?;
    let mut buf = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ConfinedError::Read(format!("{e}")))?;
    if buf.len() > MAX_CONFIG_BYTES {
        return Err(ConfinedError::TooLarge);
    }
    Ok(buf)
}

/// Read an untrusted config file through the full confinement boundary and parse
/// it: read the bytes through the `dir` capability ([`read_confined`]), strip
/// them to inert text in the S18-B parse sandbox at `sandbox_bin`, then parse
/// that text with the handler for `format`.
///
/// Returns a read-only [`ConfigModel`]; the editing entry points
/// ([`checked_set`](crate::checked_set) / [`checked_remove`](crate::checked_remove))
/// operate on text a caller already holds, not on a sandboxed read, so they do
/// not pass through here.
pub fn read_and_parse_confined(
    dir: &Dir,
    rel: impl AsRef<Path>,
    sandbox_bin: &Path,
    format: Format,
) -> Result<ConfigModel, ConfinedError> {
    let bytes = read_confined(dir, rel)?;
    let inert = arlen_ai_sandbox::parse_document(sandbox_bin, &bytes)
        .map_err(|e| ConfinedError::Sandbox(format!("{e}")))?;
    let model = handler_for(format).read(&inert)?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;

    #[test]
    fn read_confined_returns_file_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("app.toml"), b"name = \"arlen\"\n").unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let bytes = read_confined(&dir, "app.toml").unwrap();
        assert_eq!(bytes, b"name = \"arlen\"\n");
    }

    #[test]
    fn read_confined_refuses_a_traversal_escape() {
        let outer = tempfile::tempdir().unwrap();
        std::fs::write(outer.path().join("secret"), b"top secret\n").unwrap();
        let inner = outer.path().join("sub");
        std::fs::create_dir(&inner).unwrap();
        let dir = Dir::open_ambient_dir(&inner, ambient_authority()).unwrap();
        // The capability is rooted at `sub`; reaching the sibling `secret`
        // through `..` must be refused, not followed.
        let err = read_confined(&dir, "../secret").unwrap_err();
        assert!(matches!(err, ConfinedError::Read(_)));
    }

    #[test]
    fn read_confined_refuses_an_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let err = read_confined(&dir, "/etc/hostname").unwrap_err();
        assert!(matches!(err, ConfinedError::Read(_)));
    }

    #[test]
    fn read_confined_refuses_an_oversize_file() {
        let tmp = tempfile::tempdir().unwrap();
        let big = vec![b'x'; MAX_CONFIG_BYTES + 1];
        std::fs::write(tmp.path().join("big.conf"), &big).unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let err = read_confined(&dir, "big.conf").unwrap_err();
        assert!(matches!(err, ConfinedError::TooLarge));
    }
}
