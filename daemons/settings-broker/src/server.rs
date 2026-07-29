//! The socket: framing, and the lock that makes writes serial.
//!
//! Two settings pages, or a page and an app, can ask for a write at the same
//! moment. Each write is a read-modify-write of one TOML file, so two of them
//! interleaving would let the second overwrite the first's edit with a document
//! it parsed before that edit existed - a lost update, silently. The broker
//! holds a lock across the whole decide-apply-answer step so writes are strictly
//! ordered, which is the concrete reason this daemon exists rather than every
//! caller editing the file itself.
//!
//! Reads take no part in this. An app reads its own config directly, which is
//! why dconf can describe its service as involved only in writes.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::protocol::{Request, Response, MAX_FRAME};
use crate::serve::{answer, AppRegistry};

/// A framing or transport failure on one connection.
#[derive(Debug)]
pub enum ServeError {
    /// The connection broke.
    Io(std::io::Error),
    /// A frame length outside the agreed bound, refused BEFORE allocating.
    Frame(String),
    /// The body was not a valid request, or a response could not be encoded.
    Codec(String),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Io(e) => write!(f, "broker io: {e}"),
            ServeError::Frame(e) => write!(f, "broker frame: {e}"),
            ServeError::Codec(e) => write!(f, "broker codec: {e}"),
        }
    }
}

impl std::error::Error for ServeError {}

impl From<std::io::Error> for ServeError {
    fn from(e: std::io::Error) -> Self {
        ServeError::Io(e)
    }
}

/// The socket path the broker serves on.
pub fn socket_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("arlen/settings-broker.sock"))
}

/// Read one length-prefixed request. `Ok(None)` on a clean close at a frame
/// boundary, so a caller that simply hangs up is not an error.
async fn read_request<S>(stream: &mut S) -> Result<Option<Request>, ServeError>
where
    S: AsyncReadExt + Unpin,
{
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(ServeError::Io(e)),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(ServeError::Frame(format!(
            "request length {len} out of range (1..={MAX_FRAME})"
        )));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| ServeError::Codec(e.to_string()))
}

/// Write one length-prefixed response.
async fn write_response<S>(stream: &mut S, response: &Response) -> Result<(), ServeError>
where
    S: AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(response).map_err(|e| ServeError::Codec(e.to_string()))?;
    let len = u32::try_from(body.len())
        .map_err(|_| ServeError::Frame("response exceeds u32 length".to_string()))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

/// Serve one connection until the caller hangs up.
///
/// `write_lock` is shared across every connection: it is what makes concurrent
/// writes serial rather than interleaved. It is held across the answer, so the
/// read-modify-write of the config file cannot overlap another.
pub async fn serve_connection<S>(
    stream: &mut S,
    registry: &dyn AppRegistry,
    write_lock: &Mutex<()>,
) -> Result<(), ServeError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    while let Some(request) = read_request(stream).await? {
        let response = {
            let _guard = write_lock.lock().await;
            answer(registry, request)
        };
        write_response(stream, &response).await?;
    }
    Ok(())
}

/// A lock shared by every connection the broker serves.
pub fn shared_write_lock() -> Arc<Mutex<()>> {
    Arc::new(Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::KeyWrite;
    use crate::serve::AppSettings;
    use arlen_forage_recipe::settings::{
        SettingScope, SettingType, SettingsItem, SettingsSchema, SettingsSection,
    };
    use std::path::PathBuf;
    use toml::Value;

    struct OneApp {
        schema: SettingsSchema,
        path: PathBuf,
    }

    impl AppRegistry for OneApp {
        fn lookup(&self, app_id: &str) -> Option<AppSettings> {
            (app_id == "org.example.App").then(|| AppSettings {
                schema: self.schema.clone(),
                config_path: self.path.clone(),
            })
        }
    }

    fn schema() -> SettingsSchema {
        SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items: vec![SettingsItem {
                    key: "theme".into(),
                    value_type: SettingType::String,
                    label: "Theme".into(),
                    description: None,
                    default: None,
                    min: None,
                    max: None,
                    unit: None,
                    options: Vec::new(),
                    order: None,
                    keywords: Vec::new(),
                    scope: SettingScope::default(),
                    tags: Vec::new(),
                    included: None,
                    deprecated_message: None,
                    replaced_by: None,
                    renamed_from: Vec::new(),
                    since: None,
                    removed_in: None,
                    visible_when: None,
                }],
            }],
        }
    }

    async fn send<S: AsyncWriteExt + Unpin>(stream: &mut S, request: &Request) {
        let body = serde_json::to_vec(request).unwrap();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
        stream.flush().await.unwrap();
    }

    async fn recv<S: AsyncReadExt + Unpin>(stream: &mut S) -> Response {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.unwrap();
        let len = u32::from_be_bytes(header) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn write_req(key: &str, value: Value) -> Request {
        Request::Write {
            app_id: "org.example.App".into(),
            writes: vec![KeyWrite {
                key: key.into(),
                value,
            }],
        }
    }

    /// A real request and response over the framing: a wrong length width or
    /// endianness would break this.
    #[tokio::test]
    async fn a_write_round_trips_over_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let app = OneApp {
            schema: schema(),
            path: path.clone(),
        };
        let lock = shared_write_lock();
        let handle = tokio::spawn({
            let lock = lock.clone();
            async move {
                let _ = serve_connection(&mut server, &app, &lock).await;
            }
        });

        send(&mut client, &write_req("theme", Value::String("dark".into()))).await;
        match recv(&mut client).await {
            Response::Changed { changed, .. } => assert_eq!(changed, vec!["theme".to_string()]),
            other => panic!("expected Changed, got {other:?}"),
        }

        drop(client);
        handle.await.unwrap();
        // The write really reached the file, not just the response.
        let parsed: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["theme"].as_str(), Some("dark"));
    }

    /// Several requests on one connection are answered in order.
    #[tokio::test]
    async fn a_connection_serves_requests_until_the_caller_hangs_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let app = OneApp {
            schema: schema(),
            path,
        };
        let lock = shared_write_lock();
        let handle = tokio::spawn({
            let lock = lock.clone();
            async move {
                let _ = serve_connection(&mut server, &app, &lock).await;
            }
        });

        send(&mut client, &write_req("theme", Value::String("dark".into()))).await;
        let first = recv(&mut client).await;
        send(&mut client, &write_req("theme", Value::String("light".into()))).await;
        let second = recv(&mut client).await;

        match (first, second) {
            (Response::Changed { changed: a, .. }, Response::Changed { changed: b, .. }) => {
                assert_eq!(a, vec!["theme".to_string()]);
                assert_eq!(b, vec!["theme".to_string()]);
            }
            other => panic!("unexpected {other:?}"),
        }
        drop(client);
        handle.await.unwrap();
    }

    /// A length header outside the bound is refused before any allocation.
    #[tokio::test]
    async fn an_oversized_frame_is_refused() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let dir = tempfile::tempdir().unwrap();
        let app = OneApp {
            schema: schema(),
            path: dir.path().join("config.toml"),
        };
        let lock = shared_write_lock();

        let handle = tokio::spawn({
            let lock = lock.clone();
            async move { serve_connection(&mut server, &app, &lock).await }
        });

        client
            .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
            .await
            .unwrap();
        client.flush().await.unwrap();

        let result = handle.await.unwrap();
        assert!(matches!(result, Err(ServeError::Frame(_))), "{result:?}");
    }

    /// Concurrent writers must not lose an update. Each connection writes its
    /// own key; with the lock held across the read-modify-write, both survive.
    #[tokio::test]
    async fn concurrent_writers_do_not_lose_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        let schema = SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items: (0..8)
                    .map(|i| SettingsItem {
                        key: format!("k{i}"),
                        value_type: SettingType::Int,
                        label: "L".into(),
                        description: None,
                        default: None,
                        min: None,
                        max: None,
                        unit: None,
                        options: Vec::new(),
                        order: None,
                        keywords: Vec::new(),
                        scope: SettingScope::default(),
                        tags: Vec::new(),
                        included: None,
                        deprecated_message: None,
                        replaced_by: None,
                        renamed_from: Vec::new(),
                        since: None,
                        removed_in: None,
                        visible_when: None,
                    })
                    .collect(),
            }],
        };

        let lock = shared_write_lock();
        let mut tasks = Vec::new();
        for i in 0..8 {
            let (mut client, mut server) = tokio::io::duplex(64 * 1024);
            let app = OneApp {
                schema: schema.clone(),
                path: path.clone(),
            };
            let lock = lock.clone();
            tasks.push(tokio::spawn(async move {
                let served = tokio::spawn(async move {
                    let _ = serve_connection(&mut server, &app, &lock).await;
                });
                let request = Request::Write {
                    app_id: "org.example.App".into(),
                    writes: vec![KeyWrite {
                        key: format!("k{i}"),
                        value: Value::Integer(i),
                    }],
                };
                send(&mut client, &request).await;
                let response = recv(&mut client).await;
                drop(client);
                let _ = served.await;
                response
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        // Every key must be present: a lost update would drop one.
        let parsed: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for i in 0..8 {
            assert_eq!(
                parsed[format!("k{i}")].as_integer(),
                Some(i),
                "k{i} was lost; file is {parsed:?}"
            );
        }
    }
}
