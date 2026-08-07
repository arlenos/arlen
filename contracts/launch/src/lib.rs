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
        /// Its MIME type. The caller supplies it because MIME detection is
        /// shared-mime-info's job and the caller usually has the answer already;
        /// a caller that does not can ask the type separately rather than have
        /// this interface grow a sniffing mode.
        mime: String,
    },
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
    /// The request was refused. The reason is deliberately coarse: a caller
    /// learning exactly which check it failed learns how to pass it.
    Refused,
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
            mime: "image/png".into(),
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
}
