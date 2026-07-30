//! The terminal side of the read half: the request/response shape a consented
//! reader speaks, and the handler that answers it.
//!
//! The MCP read tool cannot hold blocks - it is a separate process, and blocks
//! live in the terminal app's memory. So the terminal serves them, and this is
//! the part of that server which does not need a socket: the wire types and the
//! decision, over a [`BlockSource`] the app implements against its live state.
//!
//! Keeping it here rather than in `src-tauri` is deliberate. The app crate needs
//! webkit and is outside the CI matrix, so anything that lands there is built by
//! nobody until someone runs the app; this crate is built and tested on every
//! change. The edge left for `src-tauri` is the listener and the real block
//! store, which cannot be tested without a running terminal anyway.
//!
//! Scoping is layered rather than trusted: the caller names a terminal, this
//! module asks the source for exactly that terminal's blocks, and
//! [`read_scope`](crate::read_scope) decides what of it is visible. A source that
//! returns nothing for an unknown terminal is indistinguishable from one with no
//! blocks, which is the right answer - a reader learns nothing about which
//! terminals exist.

use serde::{Deserialize, Serialize};

use crate::read_scope::{self, ReadScope};
use crate::Block;

/// What a consented reader asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    /// Which terminal to read. The consent names one; there is no "all".
    pub terminal_id: String,
    /// How many blocks, newest first. Clamped by [`read_scope::MAX_BLOCKS`].
    pub limit: usize,
    /// Whether the user's own typed commands are included (a wider consent).
    #[serde(default)]
    pub include_user_blocks: bool,
    /// Whether a still-running block is included.
    #[serde(default)]
    pub include_running: bool,
    /// The consent token proving the user approved THIS reading. Opaque here:
    /// the core does not parse or trust it, it only hands it to a
    /// [`ConsentVerifier`] that does.
    pub consent: String,
}

impl ReadRequest {
    /// The scope this request asks for. Deliberately a conversion rather than a
    /// field: the request is untrusted input and the scope is what the handler
    /// enforces, so the two stay distinct types even though they carry the same
    /// three values today.
    pub fn scope(&self) -> ReadScope {
        ReadScope {
            limit: self.limit,
            include_user_blocks: self.include_user_blocks,
            include_running: self.include_running,
        }
    }
}

/// One block as a reader sees it: the block itself, unchanged.
///
/// The response carries whole [`Block`]s rather than a reduced projection because
/// the block model is already the contract the terminal renders from, and a
/// second shape would drift from it. What a reader may see is decided by which
/// blocks come back, not by trimming fields off them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadResponse {
    /// The visible blocks, newest first.
    pub blocks: Vec<Block>,
}

/// What the socket answers: the blocks, or a refusal.
///
/// Refusal is a distinct answer rather than an empty block list, because the two
/// mean different things to the caller and conflating them would be unkind rather
/// than safe. "Nothing you may see" and "you are not allowed to ask" lead to
/// different next steps: the first is an answer, the second means the assistant
/// should ask the user to approve. Nor is the distinction an oracle - a caller
/// already knows whether it presented a token - so hiding it would cost clarity
/// and buy nothing.
///
/// The refusal deliberately carries no reason. Which of expired, wrong scope,
/// wrong terminal or malformed applies is exactly the detail that would let a
/// caller probe, and the assistant's next step is the same in every case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ReadReply {
    /// The reading was authorized; these are the visible blocks, newest first.
    Blocks(ReadResponse),
    /// The consent token does not authorize this reading.
    Refused,
}

/// Whether a presented consent token authorizes exactly the reading a request
/// asks for.
///
/// A trait rather than a direct call so this crate stays what it says it is: the
/// embeddable contract types, serde and nothing else. The real implementation
/// lives with the socket and verifies the biscuit against the daemon's published
/// key; tests implement it in a line.
pub trait ConsentVerifier {
    /// `true` only when the token authorizes this exact terminal AND scope, now.
    /// Anything else - malformed, expired, minted for a different or wider
    /// reading - is `false`.
    fn authorizes(&self, req: &ReadRequest) -> bool;
}

/// A read that has been authorized. The only way to obtain one is [`authorize`],
/// and [`handle`] takes one rather than a bare request, so serving an unverified
/// read is not something a caller can express.
///
/// Same sealed shape as `TrustedActionSchema` and `ExecutedWrite` elsewhere in the
/// tree: the check is not a step someone can forget to call, it is the only door.
pub struct AuthorizedRead<'a>(&'a ReadRequest);

/// Authorize a request, or refuse it. `None` means the token does not cover this
/// exact reading and nothing should be served.
pub fn authorize<'a, V: ConsentVerifier>(
    req: &'a ReadRequest,
    verifier: &V,
) -> Option<AuthorizedRead<'a>> {
    verifier.authorizes(req).then_some(AuthorizedRead(req))
}

/// Where the handler gets a terminal's blocks. The app implements this over its
/// live state; tests implement it over a vector.
pub trait BlockSource {
    /// The blocks of `terminal_id`, oldest first, or empty when there is no such
    /// terminal. Empty and unknown are the same answer on purpose: a reader must
    /// not be able to probe which terminals exist.
    fn blocks_for(&self, terminal_id: &str) -> Vec<Block>;
}

/// Answer a read request against `source`.
///
/// The whole decision is: ask the source for the named terminal, then let the
/// scope rules filter and bound it. No branch here can widen what
/// [`read_scope::select`] allows.
pub fn handle<S: BlockSource>(source: &S, read: &AuthorizedRead) -> ReadResponse {
    let req = read.0;
    let blocks = source.blocks_for(&req.terminal_id);
    let visible = read_scope::select(&blocks, &req.scope());
    ReadResponse { blocks: visible.into_iter().cloned().collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockBodyKind, Origin};

    struct Fake(Vec<(&'static str, Vec<Block>)>);

    impl BlockSource for Fake {
        fn blocks_for(&self, terminal_id: &str) -> Vec<Block> {
            self.0
                .iter()
                .find(|(id, _)| *id == terminal_id)
                .map(|(_, b)| b.clone())
                .unwrap_or_default()
        }
    }

    fn block(id: &str, origin: Origin) -> Block {
        Block {
            id: id.to_string(),
            command: format!("cmd-{id}"),
            exit_code: Some(0),
            duration_ms: Some(1),
            cwd: "/w".to_string(),
            git: None,
            origin,
            body_kind: BlockBodyKind::Grid,
            body: serde_json::Value::Null,
        }
    }

    fn req(terminal: &str) -> ReadRequest {
        ReadRequest {
            terminal_id: terminal.to_string(),
            limit: 5,
            include_user_blocks: false,
            include_running: false,
            consent: "token".to_string(),
        }
    }

    /// Approves everything, so a test exercises the handler rather than the
    /// verifier.
    struct Consenting;
    impl ConsentVerifier for Consenting {
        fn authorizes(&self, _req: &ReadRequest) -> bool {
            true
        }
    }

    /// Refuses everything.
    struct Refusing;
    impl ConsentVerifier for Refusing {
        fn authorizes(&self, _req: &ReadRequest) -> bool {
            false
        }
    }

    /// The request as an authorized read, for tests about the handler itself.
    fn ok(req: &ReadRequest) -> AuthorizedRead<'_> {
        authorize(req, &Consenting).expect("the consenting verifier authorizes")
    }

    #[test]
    fn a_read_returns_only_the_named_terminals_blocks() {
        let src = Fake(vec![
            ("t1", vec![block("a", Origin::Agent)]),
            ("t2", vec![block("b", Origin::Agent)]),
        ]);
        let r = req("t1");
        let got = handle(&src, &ok(&r));
        assert_eq!(got.blocks.len(), 1);
        assert_eq!(got.blocks[0].id, "a", "a read must not cross terminals");
    }

    #[test]
    fn an_unknown_terminal_reads_the_same_as_an_empty_one() {
        // Indistinguishable on purpose: a reader cannot probe which terminals
        // exist by comparing the two answers.
        let src = Fake(vec![("t1", Vec::new())]);
        let (a, b) = (req("t1"), req("nope"));
        assert_eq!(handle(&src, &ok(&a)), handle(&src, &ok(&b)));
    }

    #[test]
    fn the_handler_cannot_widen_what_the_scope_allows() {
        // The source offers the user's blocks; the default scope hides them, and
        // nothing in the handler puts them back.
        let src = Fake(vec![("t1", vec![block("u", Origin::You), block("a", Origin::Agent)])]);
        let r = req("t1");
        let got = handle(&src, &ok(&r));
        assert_eq!(got.blocks.len(), 1);
        assert_eq!(got.blocks[0].origin, Origin::Agent);
    }

    #[test]
    fn a_request_asking_for_everything_still_gets_the_cap() {
        let many: Vec<Block> =
            (0..100).map(|i| block(&i.to_string(), Origin::Agent)).collect();
        let src = Fake(vec![("t1", many)]);
        let mut r = req("t1");
        r.limit = usize::MAX;
        assert_eq!(handle(&src, &ok(&r)).blocks.len(), read_scope::MAX_BLOCKS);
    }

    #[test]
    fn a_refusal_is_distinguishable_from_an_empty_result_on_the_wire() {
        // Both are legitimate answers and they must not look alike: an empty
        // block list is "nothing you may see", a refusal is "you may not ask".
        let empty = ReadReply::Blocks(ReadResponse { blocks: Vec::new() });
        let refused = ReadReply::Refused;
        assert_ne!(
            serde_json::to_string(&empty).expect("encodes"),
            serde_json::to_string(&refused).expect("encodes"),
        );
        // And the refusal says nothing about why.
        let text = serde_json::to_string(&refused).expect("encodes");
        assert_eq!(text, r#"{"outcome":"refused"}"#);
    }

    #[test]
    fn a_refused_token_yields_no_authorized_read_at_all() {
        // The refusal is structural: without an AuthorizedRead there is nothing to
        // pass to `handle`, so a caller cannot serve an unverified request even by
        // mistake. This asserts the door is shut; the type system is what keeps it
        // shut.
        let r = req("t1");
        assert!(authorize(&r, &Refusing).is_none());
    }

    #[test]
    fn the_request_omits_the_widening_flags_by_default() {
        // A request that names only a terminal and a limit must deserialize to
        // the narrow reading, so a caller cannot widen by leaving fields out.
        let r: ReadRequest =
            serde_json::from_str(r#"{"terminal_id":"t1","limit":3,"consent":"t"}"#)
                .expect("parses");
        assert!(!r.include_user_blocks);
        assert!(!r.include_running);
    }
}
