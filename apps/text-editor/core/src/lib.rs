//! The text editor's pure core.
//!
//! Today this is the LSP client's protocol half. `text-editor-app.md` settles
//! the shape: an LSP-LITE client that "consumes external language servers it
//! doesn't ship", Helix as the model, and the line drawn at DAP. Tree-sitter and
//! a language server are both external processes the editor does not own, and
//! that externalisation is what lets it stay light.
//!
//! Pure on purpose. Framing, the handshake and document sync are decided here
//! over bytes and values, so they are tested without spawning anything; the host
//! owns the process, its confinement and its lifetime.

pub mod session;
pub mod wire;
