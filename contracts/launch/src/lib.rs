//! The launch request contract: what one component may ask another to start.
//!
//! Three components need this vocabulary - the shell, which serves the launch
//! socket; the portal, which stops spawning `xdg-open` and asks instead; and the
//! apps, whose Open-With and per-app-settings handoffs are launch requests too.
//! Shared here rather than in any one of them, because a wire type that lives in
//! one participant's crate is a dependency the other participants should not
//! have.
//!
//! **[`LaunchRequest`] cannot express a command line.** That is the point of it
//! being a type at all. A command line in a launch request is arbitrary code
//! execution wearing a request's clothes, and the moment one exists the
//! confinement flag is advisory: whoever can name a program can name
//! `sh -c ...` and confine nothing. A caller names an application, or names a
//! document and lets the system decide what opens it. Three callers remembering
//! a rule is a convention; a variant that does not exist is a guarantee.
//!
//! The resolution and the launch itself live in the shell, together, because the
//! gap this closes is that the portal knew the URI, `xdg-open` knew the handler
//! and `arlen-run` needed the app id, and nobody held all three.

use serde::{Deserialize, Serialize};

/// A document, in the two forms a desktop entry's field codes want.
///
/// Both are carried because an application declares which it takes: `%u` gets
/// the URI, `%f` the local path, and an application that only handles local
/// files cannot open a remote document at all. Deciding that at the callee, from
/// the entry, is the only place that knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The URI, for `%u` / `%U`.
    pub uri: String,
    /// The local path, for `%f` / `%F`. Absent for a document that is not a
    /// local file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What a caller is asking for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LaunchRequest {
    /// Start a named application, optionally handing it documents.
    ///
    /// The rare case, deliberately. An application that wants a *specific* other
    /// application rather than whatever opens a document is making a claim about
    /// the user's setup, and the honest default is that it does not need to.
    App {
        /// The desktop id of the application to start.
        app_id: String,
        /// Documents to hand it. Usually empty.
        #[serde(default)]
        targets: Vec<Target>,
    },
    /// Open a document with whatever the user's configuration says opens it.
    /// Nearly every real case, and the one that needs no claim about the setup.
    Open {
        /// The document.
        target: Target,
        /// Its MIME type, when the caller already knows it.
        ///
        /// **Optional because the callee is the one that has to know.** It
        /// already owns "which application opens this"; "what kind of thing is
        /// this" is the same question one step earlier, and requiring it here
        /// would put a MIME database in every caller - which is how the
        /// resolution ended up in `xdg-open` rather than beside the launch in
        /// the first place.
        ///
        /// A caller that does know says so and saves the lookup: the portal
        /// knows a `https:` link is `x-scheme-handler/https` from the URI
        /// alone, and an application that just wrote a file knows what it
        /// wrote. Absent, the service determines it from the target.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
}

/// What a caller may put on the launch socket.
///
/// A sibling of [`LaunchRequest`] rather than a variant inside it. The socket is
/// the transport; the type is the contract, and [`LaunchRequest`]'s value is what
/// it CANNOT express - a general query bolted into it would weaken exactly the
/// restraint that makes it worth being a type. A second socket was the other
/// option and costs another bind, another peer check and another gate to prove,
/// to answer one question the service already computes for itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Start something.
    Launch(LaunchRequest),
    /// Ask what kind of thing a file is.
    Query(MimeQuery),
}

/// What kind of thing is this file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MimeQuery {
    /// The local path being asked about.
    pub path: String,
}

/// The answer to a [`MimeQuery`].
///
/// **This is a read, and it is gated like one.** *What kind of file is this* leaks
/// that a path exists and what it is, so it is answered only for paths the caller
/// could have opened - the same grant that decides what it may read. Without that
/// it would be a probe telling a confined app about files it has no business
/// knowing, one type away from a request we were careful with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "answer")]
pub enum MimeAnswer {
    /// The type, as the shared-mime-info database gives it.
    Type {
        /// The MIME type.
        mime: String,
    },
    /// Outside what the caller may read.
    ///
    /// Carries a reason, because a refusal a caller cannot distinguish from a
    /// missing file is one they will retry forever. The reason names the rule, not
    /// the path - "you may not read there" tells them what to fix without
    /// confirming what is there.
    Refused {
        /// Why, in a sentence the caller can show.
        reason: String,
    },
    /// The path is readable but the database has nothing for it.
    ///
    /// Distinct from a refusal on purpose: "I may not tell you" and "there is no
    /// answer" are different facts, and collapsing them would let a caller read
    /// the grant boundary off the shape of the reply.
    Unknown,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum LaunchOutcome {
    /// The application was started.
    Started {
        /// Which application, resolved - not necessarily the one the caller
        /// named, since an `Open` request names a document.
        app_id: String,
    },
    /// Nothing is configured to open this type.
    ///
    /// Distinct from a failure on purpose: "you have not chosen a handler" is a
    /// different thing to tell someone than "it did not work", and collapsing
    /// them is how a missing default reads as a broken application.
    NoHandler {
        /// The type nothing claimed.
        mime: String,
    },
    /// The named application is not installed, or its entry could not be read.
    UnknownApplication {
        /// What was named.
        app_id: String,
    },
    /// The application's own launcher entry is not a valid command line, so its
    /// packaging is at fault rather than the request. Carried as a sentence
    /// because the caller shows it and cannot act on a code.
    MalformedEntry {
        /// Which application.
        app_id: String,
        /// What is wrong with the entry.
        reason: String,
    },
    /// The handler was found and starting it failed.
    ///
    /// The distinction [`NoHandler`] draws - "you have not chosen a handler" is
    /// a different thing to tell someone than "it did not work" - needs a second
    /// variant to be worth drawing, and this is it. Without it there was nothing
    /// honest to answer a failed spawn with, so the shell wrote nothing at all
    /// and closed the connection; a caller could then not tell a missing handler
    /// from a broken one from a dead shell.
    ///
    /// Carried as a sentence for the same reason as [`MalformedEntry`]: the
    /// caller shows it to a person, and "footclient: No such file or directory"
    /// is actionable in a way that a code is not.
    ///
    /// [`NoHandler`]: LaunchOutcome::NoHandler
    /// [`MalformedEntry`]: LaunchOutcome::MalformedEntry
    DidNotStart {
        /// Which application was resolved for the request.
        app_id: String,
        /// Why starting it failed.
        reason: String,
    },
    /// The shell put the decision in front of the person instead of starting
    /// anything.
    ///
    /// Opening a Windows executable is the case this exists for
    /// (`windows-apps-plan.md`): a `.exe` is a foreign program, and running one
    /// is a trust moment rather than a file type nobody has configured. The
    /// shell raises its own prompt - what the app is, how well it is known to
    /// work, what a fresh bottle would grant it, Run against Install - and
    /// answers here.
    ///
    /// DISTINCT FROM `Started`, because nothing has started: a caller that
    /// showed "opened" over a dialog somebody has not answered would be
    /// reporting an act they have not decided to take. Distinct from `NoHandler`
    /// for the opposite reason - something DID take the request, and telling a
    /// person nothing opens `.exe` files while a dialog about that very file is
    /// on screen is the surface contradicting itself.
    Asked {
        /// What is being asked about, for a caller that wants to say so.
        what: String,
    },
    /// The request was refused. The reason is deliberately coarse: a caller
    /// learning exactly which check it failed learns how to pass it.
    Refused,
}

/// The MIME types the shell asks about rather than hands to a handler.
///
/// Both Windows executable forms. A `.msi` is an installer and a `.exe` may be
/// either, which the prompt says rather than guesses.
pub const ASKS_FIRST: [&str; 2] = [
    "application/x-ms-dos-executable",
    "application/x-msi",
];

/// Whether a MIME type is one the shell asks about before opening.
#[must_use]
pub fn asks_first(mime: &str) -> bool {
    ASKS_FIRST.contains(&mime)
}

/// The socket the shell serves this on, per user session.
///
/// `$XDG_RUNTIME_DIR/arlen/launch.sock`, beside `notification.sock` - a session
/// service, not a system one, because the answer depends on the user's own
/// handler configuration and their session is what a launch belongs to.
pub fn socket_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/run"));
    base.join("arlen").join("launch.sock")
}

/// The largest frame either end will send or accept.
///
/// A launch request is an app id, a URI and a MIME type; 64 KiB is far past any
/// honest one, and the point of the bound is that a peer cannot make the other
/// end allocate on its say-so.
pub const MAX_FRAME: usize = 64 * 1024;

/// A framing or encoding failure on the launch socket.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    /// The stream ended, or the transport failed.
    #[error("launch socket i/o: {0}")]
    Io(#[from] std::io::Error),
    /// A frame outside the agreed bounds. Refused before allocating, so a bad
    /// length is a protocol error rather than a large read.
    #[error("launch socket framing: {0}")]
    Frame(String),
    /// The body is not a request, or not a response.
    #[error("launch socket body: {0}")]
    Body(#[from] serde_json::Error),
}

/// Read one length-prefixed frame.
///
/// Same shape as the audit socket's, deliberately: a second framing convention
/// in the same tree is a second thing to get right, and this one carries less.
pub async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>, WireError>
where
    S: tokio::io::AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(WireError::Frame(format!(
            "frame length {len} out of range (1..={MAX_FRAME})"
        )));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

/// Write one length-prefixed frame.
///
/// Bounds-checked before any byte goes out, so this end can never send a frame
/// the other end would refuse.
pub async fn write_frame<S>(stream: &mut S, body: &[u8]) -> Result<(), WireError>
where
    S: tokio::io::AsyncWriteExt + Unpin,
{
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(WireError::Frame(format!(
            "frame length {} out of range (1..={MAX_FRAME})",
            body.len()
        )));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| WireError::Frame("frame exceeds u32 length".to_string()))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

/// Send a request.
pub async fn write_request<S>(stream: &mut S, request: &LaunchRequest) -> Result<(), WireError>
where
    S: tokio::io::AsyncWriteExt + Unpin,
{
    write_frame(stream, &serde_json::to_vec(request)?).await
}

/// Receive a request.
pub async fn read_request<S>(stream: &mut S) -> Result<LaunchRequest, WireError>
where
    S: tokio::io::AsyncReadExt + Unpin,
{
    Ok(serde_json::from_slice(&read_frame(stream).await?)?)
}

/// Send an outcome.
pub async fn write_outcome<S>(stream: &mut S, outcome: &LaunchOutcome) -> Result<(), WireError>
where
    S: tokio::io::AsyncWriteExt + Unpin,
{
    write_frame(stream, &serde_json::to_vec(outcome)?).await
}

/// Receive an outcome.
pub async fn read_outcome<S>(stream: &mut S) -> Result<LaunchOutcome, WireError>
where
    S: tokio::io::AsyncReadExt + Unpin,
{
    Ok(serde_json::from_slice(&read_frame(stream).await?)?)
}

/// The MIME type that names the handler for a URI's scheme.
///
/// freedesktop models scheme handlers as MIME types - `x-scheme-handler/https`
/// for a web link, `x-scheme-handler/mailto` for an address - so a URL is opened
/// through exactly the same lookup as a document, and [`LaunchRequest::Open`]
/// needs no second shape for it. That is why a caller with a `https://` link has
/// something to put in `mime` without owning a MIME database.
///
/// `None` for `file:`, which is a real document whose type comes from its
/// content rather than its scheme, and for anything that is not a URI with a
/// scheme. A scheme is `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )` per RFC 3986;
/// anything else is refused rather than lowercased into a plausible-looking
/// handler name.
pub fn scheme_handler_mime(uri: &str) -> Option<String> {
    let scheme = uri.split_once(':')?.0;
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    if scheme == "file" {
        return None;
    }
    Some(format!("x-scheme-handler/{scheme}"))
}

/// Percent-encode a local path into the path component of a `file:` URI.
///
/// Unreserved characters and `/` pass through; everything else is encoded,
/// including the space and the `#` that would otherwise start a fragment and
/// truncate the name. Two sites in the tree build this with `format!("file://
/// {path}")`, which is fine until a filename contains one of those.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Write one [`Request`] envelope.
pub async fn write_message<S>(stream: &mut S, request: &Request) -> Result<(), WireError>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    write_frame(stream, &serde_json::to_vec(request)?).await
}

/// Read one [`Request`] envelope.
pub async fn read_message<S>(stream: &mut S) -> Result<Request, WireError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    Ok(serde_json::from_slice(&read_frame(stream).await?)?)
}

/// Write one [`MimeAnswer`].
pub async fn write_answer<S>(stream: &mut S, answer: &MimeAnswer) -> Result<(), WireError>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    write_frame(stream, &serde_json::to_vec(answer)?).await
}

/// Read one [`MimeAnswer`].
pub async fn read_answer<S>(stream: &mut S) -> Result<MimeAnswer, WireError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    Ok(serde_json::from_slice(&read_frame(stream).await?)?)
}

/// Ask the service what kind of thing a file is.
///
/// The answer is gated by what the caller may read; see [`MimeAnswer`].
pub async fn query_mime(path: &str) -> Result<MimeAnswer, WireError> {
    let mut stream = tokio::net::UnixStream::connect(socket_path()).await?;
    write_message(&mut stream, &Request::Query(MimeQuery { path: path.to_string() })).await?;
    read_answer(&mut stream).await
}

/// A local path as a `file:` URI, percent-encoded.
///
/// Public because the launch socket is not the only thing that needs one - the
/// portal's open-with chooser takes a URI too - and two encoders is how one of
/// them ends up not encoding the `#` in a filename.
pub fn file_uri(path: &str) -> String {
    format!("file://{}", encode_path(path))
}

/// Ask the launch service to act on one request, and wait for what happened.
///
/// One request per connection, which is what the service serves: connect, send,
/// read the outcome, drop. There is no session to keep.
///
/// **This is the replacement for spawning `xdg-open`, and the reason it belongs
/// here rather than in each caller.** Every participant was reimplementing the
/// same three lines around the same socket, so most of them did not, and kept
/// the subprocess instead - which is how a launch ended up with no confinement
/// flag, no resolved app id and nothing to report back but an exit status.
pub async fn request(req: &LaunchRequest) -> Result<LaunchOutcome, WireError> {
    let mut stream = tokio::net::UnixStream::connect(socket_path()).await?;
    write_message(&mut stream, &Request::Launch(req.clone())).await?;
    read_outcome(&mut stream).await
}

/// Open a local file with whatever the user configured to open it.
///
/// Carries both forms: the URI for an application declaring `%u`, the path for
/// one declaring `%f`. Which is used is the service's decision, from the
/// application's own desktop entry - the caller cannot know it and should not
/// have to guess.
pub async fn open_path(path: &str) -> Result<LaunchOutcome, WireError> {
    request(&open_path_request(path)).await
}

/// The request [`open_path`] sends, without sending it.
///
/// Public because the building is the part with a decision in it and the sending
/// is not, so this is the part worth testing and worth reading.
pub fn open_path_request(path: &str) -> LaunchRequest {
    LaunchRequest::Open {
        target: Target {
            uri: file_uri(path),
            path: Some(path.to_string()),
        },
        mime: None,
    }
}

/// Open a URI with whatever the user configured for its scheme.
///
/// The MIME goes along when the scheme names it, because for `https:` the caller
/// genuinely does know and the lookup is pure cost.
pub async fn open_uri(uri: &str) -> Result<LaunchOutcome, WireError> {
    request(&open_uri_request(uri)).await
}

/// The request [`open_uri`] sends, without sending it.
pub fn open_uri_request(uri: &str) -> LaunchRequest {
    LaunchRequest::Open {
        target: Target { uri: uri.to_string(), path: None },
        mime: scheme_handler_mime(uri),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(r: &LaunchRequest) -> LaunchRequest {
        serde_json::from_str(&serde_json::to_string(r).unwrap()).unwrap()
    }

    #[test]
    fn a_request_survives_the_wire() {
        let r = LaunchRequest::Open {
            target: Target {
                uri: "file:///tmp/a.png".into(),
                path: Some("/tmp/a.png".into()),
            },
            mime: Some("image/png".into()),
        };
        assert_eq!(round_trip(&r), r);
    }

    #[test]
    fn an_app_request_defaults_to_no_documents() {
        let parsed: LaunchRequest =
            serde_json::from_str(r#"{"kind":"app","app_id":"org.x.App"}"#).unwrap();
        assert_eq!(
            parsed,
            LaunchRequest::App {
                app_id: "org.x.App".into(),
                targets: vec![]
            }
        );
    }

    /// A remote document has no local path, and inventing one would let an
    /// application that only takes `%f` be handed something that is not a file.
    #[test]
    fn a_target_without_a_path_stays_without_one() {
        let t = Target {
            uri: "https://example.org/x".into(),
            path: None,
        };
        let back: Target = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back.path, None);
        assert!(!serde_json::to_string(&t).unwrap().contains("path"));
    }

    /// The guarantee this type exists for: there is no way to ask for a command
    /// line, so a request that tries is not a request.
    #[test]
    fn a_command_line_is_not_a_representable_request() {
        for body in [
            r#"{"kind":"exec","command":"sh -c 'rm -rf ~'"}"#,
            r#"{"kind":"app","app_id":"x","command":"sh -c x"}"#,
            r#"{"command":"sh -c x"}"#,
        ] {
            assert!(
                serde_json::from_str::<LaunchRequest>(body).is_err()
                    || !serde_json::to_string(
                        &serde_json::from_str::<LaunchRequest>(body).unwrap()
                    )
                    .unwrap()
                    .contains("command"),
                "a command line survived deserialisation: {body}"
            );
        }
    }

    /// A URL needs no second request shape: freedesktop already models a scheme
    /// handler as a MIME type.
    #[test]
    fn a_scheme_becomes_the_mime_type_that_names_its_handler() {
        assert_eq!(
            scheme_handler_mime("https://example.org/x").as_deref(),
            Some("x-scheme-handler/https")
        );
        assert_eq!(
            scheme_handler_mime("MAILTO:someone@example.org").as_deref(),
            Some("x-scheme-handler/mailto")
        );
    }

    /// A file is a document; its type comes from its content, not its scheme.
    #[test]
    fn a_file_uri_has_no_scheme_handler() {
        assert_eq!(scheme_handler_mime("file:///tmp/a.png"), None);
        assert_eq!(scheme_handler_mime("FILE:///tmp/a.png"), None);
    }

    /// Anything that is not a scheme is refused rather than lowercased into a
    /// handler name that looks real.
    #[test]
    fn a_non_scheme_is_refused() {
        for bad in [
            "/tmp/a.png",
            "no-colon",
            "1http://x",
            "sch eme://x",
            "sch/eme://x",
            ":empty",
        ] {
            assert_eq!(scheme_handler_mime(bad), None, "accepted {bad}");
        }
    }

    /// A caller that does not know the type says nothing rather than guessing,
    /// and the service works it out.
    #[test]
    fn a_request_may_omit_the_type_entirely() {
        let parsed: LaunchRequest =
            serde_json::from_str(r#"{"kind":"open","target":{"uri":"file:///tmp/a"},"mime":null}"#)
                .unwrap();
        assert!(matches!(parsed, LaunchRequest::Open { mime: None, .. }));
        let absent: LaunchRequest =
            serde_json::from_str(r#"{"kind":"open","target":{"uri":"file:///tmp/a"}}"#).unwrap();
        assert!(matches!(absent, LaunchRequest::Open { mime: None, .. }));
    }

    #[test]
    fn an_outcome_survives_the_wire() {
        let o = LaunchOutcome::NoHandler {
            mime: "application/x-nothing".into(),
        };
        let back: LaunchOutcome =
            serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert_eq!(back, o);
    }
    #[tokio::test]
    async fn a_request_and_an_outcome_survive_the_socket() {
        let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
        let sent = LaunchRequest::App {
            app_id: "org.x.App".into(),
            targets: vec![Target {
                uri: "file:///tmp/a".into(),
                path: Some("/tmp/a".into()),
            }],
        };
        write_request(&mut a, &sent).await.unwrap();
        assert_eq!(read_request(&mut b).await.unwrap(), sent);

        let answered = LaunchOutcome::Started {
            app_id: "org.x.App".into(),
        };
        write_outcome(&mut b, &answered).await.unwrap();
        assert_eq!(read_outcome(&mut a).await.unwrap(), answered);
    }

    /// A length nobody would send is refused before anything is allocated on the
    /// strength of it.
    #[tokio::test]
    async fn an_oversized_length_is_refused_without_reading_a_body() {
        let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
        use tokio::io::AsyncWriteExt;
        a.write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
            .await
            .unwrap();
        a.flush().await.unwrap();
        assert!(matches!(read_frame(&mut b).await, Err(WireError::Frame(_))));
    }

    /// And this end cannot send one either, so the two bounds cannot drift into
    /// a writer that emits what the reader refuses.
    #[tokio::test]
    async fn an_oversized_body_is_refused_before_any_byte_goes_out() {
        let (mut a, _b) = tokio::net::UnixStream::pair().unwrap();
        let big = vec![b'x'; MAX_FRAME + 1];
        assert!(matches!(
            write_frame(&mut a, &big).await,
            Err(WireError::Frame(_))
        ));
        assert!(matches!(
            write_frame(&mut a, &[]).await,
            Err(WireError::Frame(_))
        ));
    }

    /// A body that is not a request is a protocol error, not a panic.
    #[tokio::test]
    async fn a_body_that_is_not_a_request_is_an_error() {
        let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
        write_frame(&mut a, b"{\"kind\":\"nonsense\"}")
            .await
            .unwrap();
        assert!(matches!(
            read_request(&mut b).await,
            Err(WireError::Body(_))
        ));
    }

    #[test]
    fn a_path_with_a_space_or_a_hash_survives_the_uri() {
        // Unencoded, `#` starts a fragment and everything after it is dropped -
        // the file opens as a different, shorter name, or not at all.
        let LaunchRequest::Open { target, .. } = open_path_request("/home/u/my notes #2.txt")
        else {
            panic!("open_path builds an Open request");
        };
        assert_eq!(target.uri, "file:///home/u/my%20notes%20%232.txt");
        // The path stays verbatim: it is handed to an application as an argument,
        // not parsed as a URI, so encoding it would be corruption.
        assert_eq!(target.path.as_deref(), Some("/home/u/my notes #2.txt"));
    }

    #[test]
    fn an_ordinary_path_is_left_alone() {
        let LaunchRequest::Open { target, .. } = open_path_request("/home/u/a-b_c.1~/x.txt")
        else {
            panic!("open_path builds an Open request");
        };
        assert_eq!(target.uri, "file:///home/u/a-b_c.1~/x.txt");
    }

    #[test]
    fn a_uri_carries_the_mime_its_scheme_already_names() {
        let LaunchRequest::Open { target, mime } = open_uri_request("https://example.invalid/x")
        else {
            panic!("open_uri builds an Open request");
        };
        assert_eq!(target.uri, "https://example.invalid/x");
        assert_eq!(mime.as_deref(), Some("x-scheme-handler/https"));
        // No path: it is not a local file, and claiming one would offer a
        // `%f` application something it cannot open.
        assert!(target.path.is_none());
    }
}
