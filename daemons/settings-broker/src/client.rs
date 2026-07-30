//! Talking to the settings broker.
//!
//! Settings is the caller this exists for: it renders an app's page from the
//! declared schema and, when the user changes something, the write goes here
//! rather than into the app's `config.toml` directly. That is the point of a
//! broker - one process validates every write against the schema, so a page
//! cannot store a value the app never declared.
//!
//! The client is deliberately thin. It frames a request, reads the answer, and
//! hands back the broker's own [`Response`], including the refusals: a caller
//! showing the user why a value was rejected needs the broker's reason, not a
//! flattened "it failed".

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::protocol::{KeyWrite, Request, Response, MAX_FRAME};

/// Why a request did not complete. Distinct from a [`Response::Refused`], which
/// IS a completed request whose answer was no.
#[derive(Debug)]
pub enum ClientError {
    /// The broker is not running, or its socket is not reachable.
    Unreachable(std::io::Error),
    /// The connection broke mid-exchange.
    Io(std::io::Error),
    /// The broker answered with something this client cannot read.
    Malformed(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Unreachable(e) => write!(f, "the settings broker is not reachable: {e}"),
            ClientError::Io(e) => write!(f, "the settings broker connection failed: {e}"),
            ClientError::Malformed(m) => write!(f, "the settings broker answered oddly: {m}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Write one app's settings through the broker.
///
/// One call, one connection: writes are rare (a person moving a slider) and a
/// pooled connection would have to survive a broker restart to be worth
/// anything.
pub async fn write_settings(
    socket: &std::path::Path,
    app_id: &str,
    writes: Vec<KeyWrite>,
) -> Result<Response, ClientError> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(ClientError::Unreachable)?;
    exchange(
        stream,
        &Request::Write {
            app_id: app_id.to_string(),
            writes,
        },
    )
    .await
}

/// Send one request over an open connection and read the single answer.
///
/// Split out so the framing is testable against a socket pair without a running
/// broker.
pub async fn exchange<S>(mut stream: S, request: &Request) -> Result<Response, ClientError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(request)
        .map_err(|e| ClientError::Malformed(format!("could not encode the request: {e}")))?;
    if body.len() > MAX_FRAME {
        return Err(ClientError::Malformed(format!(
            "the request is {} bytes, over the {MAX_FRAME} limit",
            body.len()
        )));
    }
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .await
        .map_err(ClientError::Io)?;
    stream.write_all(&body).await.map_err(ClientError::Io)?;
    stream.flush().await.map_err(ClientError::Io)?;

    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.map_err(ClientError::Io)?;
    let len = u32::from_be_bytes(header) as usize;
    // Check the length before allocating: the answer's own header decides how
    // much this process is about to reserve.
    if len == 0 || len > MAX_FRAME {
        return Err(ClientError::Malformed(format!(
            "answer length {len} is outside 1..={MAX_FRAME}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.map_err(ClientError::Io)?;

    serde_json::from_slice(&buf)
        .map_err(|e| ClientError::Malformed(format!("could not read the answer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml::Value;

    /// Round trip against the real serve loop rather than a hand-written stub, so
    /// the two halves of the framing are proven to agree.
    #[tokio::test]
    async fn a_write_reaches_the_broker_and_the_answer_comes_back() {
        use crate::serve::{AppRegistry, AppSettings};
        use arlen_forage_recipe::settings::{
            SettingType, SettingsItem, SettingsSchema, SettingsSection,
        };

        struct One(std::path::PathBuf);
        impl AppRegistry for One {
            fn lookup(&self, app_id: &str) -> Option<AppSettings> {
                if app_id != "org.example.App" {
                    return None;
                }
                Some(AppSettings {
                    schema: SettingsSchema {
                        version: 1,
                        sections: vec![SettingsSection {
                            label: "S".into(),
                            description: None,
                            order: None,
                            items: vec![SettingsItem::new("theme", SettingType::String, "Theme")],
                        }],
                    },
                    config_path: self.0.clone(),
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "").unwrap();

        let (client, mut server) = UnixStream::pair().unwrap();
        let registry = One(config.clone());
        let lock = tokio::sync::Mutex::new(());
        tokio::spawn(async move {
            let _ = crate::server::serve_connection(&mut server, &registry, &lock, "org.example.App").await;
        });

        let answer = exchange(
            client,
            &Request::Write {
                app_id: "org.example.App".into(),
                writes: vec![KeyWrite {
                    key: "theme".into(),
                    value: Value::String("dark".into()),
                }],
            },
        )
        .await
        .expect("the exchange should complete");

        assert_eq!(
            answer,
            Response::Changed {
                app_id: "org.example.App".into(),
                changed: vec!["theme".into()],
            }
        );
        assert!(std::fs::read_to_string(&config).unwrap().contains("dark"));
    }

    /// A refusal is a completed exchange, not an error: the caller needs the
    /// broker's own reason to show the user why the value was not stored.
    #[tokio::test]
    async fn a_refusal_comes_back_as_an_answer() {
        let (client, mut server) = UnixStream::pair().unwrap();
        tokio::spawn(async move {
            let mut header = [0u8; 4];
            server.read_exact(&mut header).await.unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
            server.read_exact(&mut body).await.unwrap();

            let reply = serde_json::to_vec(&Response::Refused {
                key: "count".into(),
                reason: "expected an integer".into(),
            })
            .unwrap();
            server
                .write_all(&(reply.len() as u32).to_be_bytes())
                .await
                .unwrap();
            server.write_all(&reply).await.unwrap();
        });

        let answer = exchange(
            client,
            &Request::Write {
                app_id: "a".into(),
                writes: Vec::new(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(answer, Response::Refused { .. }));
    }

    /// The answer's own header decides how much this process allocates, so it is
    /// checked before the buffer is reserved.
    #[tokio::test]
    async fn an_oversized_answer_is_refused_before_allocating() {
        let (client, mut server) = UnixStream::pair().unwrap();
        tokio::spawn(async move {
            let mut header = [0u8; 4];
            server.read_exact(&mut header).await.unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
            server.read_exact(&mut body).await.unwrap();
            server
                .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
                .await
                .unwrap();
        });

        let err = exchange(
            client,
            &Request::Write {
                app_id: "a".into(),
                writes: Vec::new(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ClientError::Malformed(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_missing_broker_is_reported_as_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let err = write_settings(&dir.path().join("nope.sock"), "a", Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, ClientError::Unreachable(_)), "{err:?}");
    }
}
