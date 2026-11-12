//! The app's own connection point to the compositor.
//!
//! Binding the compositor's socket into the sandbox lets the app connect as
//! anybody: the compositor sees a client with no history and offers it every
//! global, screen capture included. `wp_security_context_v1` is the protocol's
//! answer, and the one flatpak uses. The launcher asks the compositor to listen
//! on a second socket and to mark every client that arrives there with a
//! sandbox engine and an app id. That socket is what gets bound into the
//! sandbox, at the path where the app expects the compositor.
//!
//! The compositor keeps listening after the launcher's own connection is gone,
//! until the read end of a pipe it was handed reports that the write end has
//! closed. [`AppDisplay`] holds the write end, so the socket lives exactly as
//! long as the launch.

use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::security_context::v1::client::{
    wp_security_context_manager_v1::{self, WpSecurityContextManagerV1},
    wp_security_context_v1::{self, WpSecurityContextV1},
};

/// The sandbox engine every context of this launcher carries. The compositor
/// reads it to tell an Arlen launch from a flatpak one.
pub const ENGINE: &str = "dev.arlen.run";

/// How long the compositor gets to answer before the launch is refused. A
/// compositor that does not answer a registry request is not one an app could
/// draw on either.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// A compositor socket made for one launch. Dropping it tells the compositor to
/// stop listening and removes the socket file.
#[derive(Debug)]
pub struct AppDisplay {
    path: PathBuf,
    /// The write end of the pipe whose read end the compositor watches.
    _open_while_held: OwnedFd,
}

impl AppDisplay {
    /// Where the socket is on the host. It is bound into the sandbox from here.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AppDisplay {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Why no socket could be made for the app.
#[derive(Debug)]
pub enum ContextError {
    /// The compositor could not be reached or stopped answering.
    Compositor(String),
    /// The socket or the pipe could not be made.
    Io(io::Error),
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextError::Compositor(e) => write!(f, "the compositor: {e}"),
            ContextError::Io(e) => write!(f, "the app's compositor socket: {e}"),
        }
    }
}

impl From<io::Error> for ContextError {
    fn from(e: io::Error) -> Self {
        ContextError::Io(e)
    }
}

/// What the registry offered.
#[derive(Default)]
struct Offered {
    manager: Option<WpSecurityContextManagerV1>,
}

impl Dispatch<WlRegistry, ()> for Offered {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            if interface == "wp_security_context_manager_v1" {
                state.manager = Some(registry.bind(name, 1, qh, ()));
            }
        }
    }
}

impl Dispatch<WpSecurityContextManagerV1, ()> for Offered {
    fn event(
        _: &mut Self,
        _: &WpSecurityContextManagerV1,
        _: wp_security_context_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpSecurityContextV1, ()> for Offered {
    fn event(
        _: &mut Self,
        _: &WpSecurityContextV1,
        _: wp_security_context_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Where the launch's socket goes: its own directory in the runtime dir, next
/// to and not inside `arlen/`, which is bound whole into every sandbox. A
/// context socket in there would let one app connect under another's app id.
pub fn socket_path(runtime_dir: &Path, launch_pid: u32) -> PathBuf {
    runtime_dir.join("arlen-run").join(format!("{launch_pid}.wayland"))
}

/// Ask the compositor at `compositor` for a socket whose clients are marked as
/// `app_id`, launched by this engine, instance `launch_pid`.
///
/// `Ok(None)` when the compositor does not speak the protocol: there is nothing
/// to mark a client with, and the caller decides what the app gets. An error
/// when it does speak it and the socket still could not be made, which the
/// caller treats as a failed launch.
pub fn open(
    compositor: &Path,
    runtime_dir: &Path,
    app_id: &str,
    launch_pid: u32,
) -> Result<Option<AppDisplay>, ContextError> {
    let stream = UnixStream::connect(compositor)?;
    stream.set_read_timeout(Some(ANSWER_WITHIN))?;
    let conn =
        Connection::from_socket(stream).map_err(|e| ContextError::Compositor(e.to_string()))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut offered = Offered::default();
    queue
        .roundtrip(&mut offered)
        .map_err(|e| ContextError::Compositor(e.to_string()))?;
    let Some(manager) = offered.manager.take() else {
        return Ok(None);
    };

    let path = socket_path(runtime_dir, launch_pid);
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    // A launcher pid that was used before and did not clean up.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let (close_read, close_write) = std::io::pipe()?;
    let display = AppDisplay { path, _open_while_held: close_write.into() };

    let context = manager.create_listener(listener.as_fd(), close_read.as_fd(), &qh, ());
    context.set_sandbox_engine(ENGINE.to_string());
    context.set_app_id(app_id.to_string());
    context.set_instance_id(launch_pid.to_string());
    context.commit();
    // The roundtrip is what tells a refusal from an acceptance: a protocol
    // error on any of the four requests comes back here.
    queue
        .roundtrip(&mut offered)
        .map_err(|e| ContextError::Compositor(e.to_string()))?;
    Ok(Some(display))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_beside_the_shared_directory_not_in_it() {
        let path = socket_path(Path::new("/run/user/1000"), 4321);
        assert_eq!(path, PathBuf::from("/run/user/1000/arlen-run/4321.wayland"));
        assert!(!path.starts_with("/run/user/1000/arlen/"));
    }

    #[test]
    fn a_compositor_that_is_not_there_is_an_error_not_a_missing_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let got = open(&dir.path().join("wayland-9"), dir.path(), "dev.example.app", 1);
        assert!(matches!(got, Err(ContextError::Io(_))));
    }
}
